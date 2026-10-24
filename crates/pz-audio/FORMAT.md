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
