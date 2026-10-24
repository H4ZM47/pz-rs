# FMOD Studio bank format notes (bank version 0x92)

Working notes from reverse engineering, kept with the code so later work does
not start over. Field names marked `?` are guesses; "fits" means the layout
parses every chunk of that type in the game's banks with no bytes left over.
Only the game's 0x92 banks matter (`ZomboidSound`, `ZomboidMusic`,
`ZomboidSoundMP`, `ZomboidSound.strings`); `radio.bank` (0x87) and
`export/Desktop/zombies.bank` (0x44) are not loaded by the game.

## Container

RIFF `FEV ` with `FMT ` (two u32: format version, minimum version), then
`LIST PROJ` holding one list per object kind (`LIST EVTS` → `LIST EVNT` →
`EVTB`, `PRPS`, ...), and `SND ` with the FSB5 at the next 32-byte-aligned
file offset. `LCNT` in each list holds the u32 element count.

## Serialisation primitives

* count: u16; if bit 15 is set, a second u16 supplies bits 15 and up.
* array of structures: `count << 1 | 1`, then a u16 element size, then the
  elements; an empty array is just `01 00` (no size). The other form is
  `count << 1` with a u16 size before each element.
* embedded structure: u16 size, then that many bytes.
* string: u16 byte length, bytes, no terminator.
* GUID: 16 bytes, Data1..Data3 little endian.
* Times are in samples at 48 kHz. Event length (ms) = max end / 48.

## Objects (fits unless noted)

* `STDT` (strings bank): radix tree, see `src/strings.rs`. Matches FMOD's
  `Bank::getStringInfo` exactly.
* `WAV `: GUID, embedded {u32 ?, u32 FSB5 sample index, u32 flags (0x20000
  on streamed waveforms)}.
* `WAIB` (single sound instrument): GUID, GUID of `WAV `.
* `MUIB` (multi instrument): GUID. Its playlist is `PLST`: u32 play mode?,
  u32 ?, array of {instrument GUID, f32 weight %}.
* `EVTB` (event): GUID, parent GUID (snapshot record for snapshots), timeline
  GUID (`TLNB`), input bus (`IBSB`), master bus (`MBSB`), u32 max instances,
  u32 ?, u32 ?, u8 ?, array of parameter-sheet GUIDs (`PMLB`), u32 ×5 (last is
  a flags word), array of track GUIDs (`GBSB`), array of parameter IDs
  (u32, u32), array of instrument GUIDs, f32 min distance, f32 max distance.
* `PRMB` (parameter): GUID, u32 flags (1 global, 8 discrete, 0x10 automatic,
  0x20 labelled, 2 ?), u32 type (FMOD_STUDIO_PARAMETER_TYPE), string name,
  f32 min, f32 max, f32 default, f32 ?, array of labels (strings). API flags:
  global → GLOBAL|READONLY, automatic → AUTOMATIC|READONLY.
  Parameter IDs are not in `PRMB`; events carry them.
* `TLNB` (timeline): GUID, array of {instrument GUID, u32 start, u32 length},
  a second array of the same shape, array ? (tempo markers?), array of
  markers {GUID, u32 position, string name, u32 length (regions)}, array ?.
* `BUS ` (every bus): u32 ?, u32 speaker mode, array of effect GUIDs, array of
  effect GUIDs, embedded {f32, f32}, array of VCA GUIDs, u32 0x7fffffff,
  u32 ?, array of u32, array of u32, i32 -1, u32 0.
* `GBSB`/`MBSB`/`IBSB`/`RBSB`: GUID, embedded {output bus GUID, u32 speaker
  mode, i32 -1}.
* `INST` (shared instrument fields, 130 bytes in 0x92): GUID, f32 volume dB,
  f32 ?, i32 loop count (-1 forever), u32 ?, f32 ?, f32 probability %,
  f32 delay min ms?, f32 delay max ms?, ..., f32 1.0 @0x48, u32 max voices
  @0x50, embedded {output GUID, u32 speaker mode, i32 -1} @0x54, ...

## Oracle

The FMOD runtime shipped with the game (FMOD 2.03.09, Studio header version
0x00020309) can be driven through ctypes for parity checks; keep such
scripts and their output outside the repository.

## Automation

* `CTRL`: GUID, target object GUID, curve GUID, u32 property id.
* `CURV`: GUID, controller GUID, array of points {f32 x, f32 y, f32 shape,
  u32 flags}. Shapes are non-zero on about a fifth of points (flags 3).
* `MAP `: GUID, array of (f32, f32): maps the automation scale to the
  property's value (fader display scale to dB, distance scale to metres).
* `PRPS` → `PROP` on any object: u32 property id (0 volume on buses and
  instruments; effect parameters are 1000 + DSP parameter index), u16, u16,
  `MAP` GUID, array of curve GUIDs, array of modulator GUIDs.
* `CTRO` beside `PMLB` (and `TLNB`): the controllers that parameter sheet
  (timeline) drives. A curve's x axis is that parameter's value.
* Runtime (current approximation): automated value = map(sum of the
  property's active curves), replacing the base value.

## Effects

Effect objects (`BEFF`) hold `BEFB` (GUID, u32 Studio effect type), `PMEF`
(u32 parameter count, then per parameter u32 kind and a value: 0 float f32,
1 int i32, 2 bool u8, 3 data u16 size + bytes; one trailing u8) and `EFFB`
(u32 bypass flag, f32 100, f32 0, f32 -80, u32 0). Parameters are in the
order of the matching core DSP's parameters (FMOD_DSP_<TYPE>_* indices).
Studio type → core DSP, from parameter counts and values:

| Studio | Core DSP | Uses in the game's banks |
| --- | --- | --- |
| 6 | Echo | 6 |
| 7 | Fader | 266 |
| 11 | Limiter | 1 (master bus) |
| 18 | Compressor | 39 |
| 19 | SFX Reverb | 8 |
| 27 | Pan (spatializer) | 73 |
| 28 | 3-EQ | 1 |
| 30 | Loudness Meter | 1 |
| 34 | Transceiver | 21 |
| 36 | Multiband Equalizer | 149 |

Send effects (`SEFF`: `SEFB` GUID, return bus GUID, f32 level dB) route to
return buses. Buses carry `PRPS` lists whose `PROP` entries automate
properties through `MAP` and `CURV` objects.

The game's master bus runs a fader, compressor, limiter and loudness meter,
so every event's level depends on them.

### Spatializer (Pan) measurements

Studio's spatializer is the core Pan DSP in surround mode into 5.1:
point sources use 2D VBAP over C 0°, FR/FL ±30°, SR/SL ±110°; rolloffs
are linear-squared, linear, inverse (clamped at max), inverse-tapered
(min of inverse and linear-squared) and custom (none); the 5.1 → stereo
output downmix is L = FL + C/√2 + SL/√2. Extent spreading, sound size,
elevation and gain smoothing are approximated (see `src/dsp/pan.rs`).

### Measured behaviour (core DSPs driven through the oracle)

* Fader: exact dB gain.
* Compressor, limiter, multiband EQ and reverb are not implemented in this
  tree yet; they pass the signal through, so levels through the master bus
  differ from FMOD's.
