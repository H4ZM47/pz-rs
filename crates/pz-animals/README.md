# pz-animals

Animals (`zombie.characters.animals`). This crate is step 1 of the port: the
definition tables and the genome rules. All data is read at runtime from the
game's own Lua. Nothing is hard-coded.

| Original | Rust | Data source |
|---|---|---|
| `AnimalDefinitions` (+ `datas/AnimalBreed`, `datas/AnimalGrowStage`) | `definitions.rs`, `breed.rs` | global `AnimalDefinitions.animals` (`media/lua/shared/Definitions/animal/<Animal>Definitions.lua`) |
| `AnimalGenomeDefinitions` | `genome.rs` (`AnimalGenomeDefinitions`, `GeneDefinition`) | global `AnimalGenomeDefinitions.genes` / `.geneticDisorder` (`AnimalGenomeDefinitions.lua`) |
| `AnimalGene`, `AnimalAllele` | `genome.rs` (`init_genome`, `init_genes_from_parents`, `do_mutation`, `do_ratio`, `check_genetic_disorder`) | — |
| `AnimalPartsDefinitions`, `AnimalPart` | `parts.rs` | global `AnimalPartsDefinitions.animals` (`AnimalPartsDefinitions.lua`), read on every call |
| `AnimalTracksDefinitions` (+ `AnimalTracksType`) | `tracks.rs` | global `AnimalTracksDefinitions.animallist` (`AnimalTracksDefinitions.lua`) |
| `MigrationGroupDefinitions` | `migration.rs` | global `MigrationGroupDefinitions` (`MigrationGroupDefinitions.lua`) |

How the loaders work:

* They read any Kahlua environment. In the game that is `LuaManager.env`
  after `LoadDirBase("shared")`.
* `lua::DefinitionsLua` is a small stand-alone environment for tools and
  tests. It runs only `media/lua/shared/Definitions/animal/*` in
  `LoadDirBase` order, with the stdlib, `stdlib.lua` and `copyTable`. No
  other shared file writes these globals.
  * `AnimalAvatarDefinition.lua` fails to run in it, because it needs the
    exposed `IsoDirections` class. That file sets none of the tables above.

Behaviour kept from the original:

* **Iteration order.** Kahlua tables iterate in insertion order. Where the
  original stores something in a `java.util.HashMap<String, _>`, the
  `java::JHashMap` type iterates in the original's bucket order. This covers
  `animalDefs`, `fullGenomeDef`, `ratios`, `forcedGenes`, breed `sounds`,
  `tracksDefinitions`, `tracks`, `trackChance`, `migrationDef`, and an
  animal's `fullGenome`.
* **`AnimalTracksType.sprites`.** It is keyed by `IsoDirections`, an enum
  whose hash is the identity hash. Its order is not reproducible, so it is
  kept in insertion order.
* **Parsing.**
  * Values print as Java `toString` (`Double.toString` for numbers).
  * Numbers go through `Float.parseFloat` and `Float.valueOf(..).intValue()`.
  * Lists go through `String.split(",")`.
  * Booleans go through `Boolean.parseBoolean`.
  * Keys are compared with `equalsIgnoreCase`.
* **Errors.** An exception the original would throw (a bad number, or a
  non-table where a table is cast) is returned as `JavaException` and aborts
  the load.
* **`Model` fields.** The `Model` fields that `ModelManager` resolves after
  loading are left to the renderer. Their `*Str` names are kept.

`AnimalDefinitions::to_save_definition` produces the subset that
`pz_world::save::animal` reads when it loads saved animals.

## Parity

`examples/animal_defs_parity.rs <oracle dump> <game dir>` compares against a
dump made by the original loaders. The oracle and its data are local only
and never committed. The oracle is a headless JVM: `ZomboidFileSystem` and
`LuaManager.init` with a scratch HOME, then `LoadDirBase("shared")`, then
`loadAnimalDefinitions`, `loadTracksDefinitions` and
`loadMigrationsDefinitions`.

The comparison covers:

* every field of every definition, by reflection, with floats compared as
  their bits;
* `AnimalPartsDefinitions` for every type;
* `AnimalGene.initGenome` for every definition × breed with a seeded `Rand`;
* `initGenesFromParents` of consecutive genomes.

Result on B42: 275 of 275 lines are identical, with 0 differences. The lines
are 18 gene definitions, 1 disorder list, 30 animal definitions (63 breeds in
total), 2 track definitions, 5 migration groups, 94 parts lookups, 63 genome
generations and 62 inheritances.

## Plan

The remaining steps, with what each needs and how each is checked.
Line references are to the decompiled sources (`zombie/...`).

### Crate layering

* `pz-animals` sits **below** `pz-characters`, the way `pz-popman` and
  `pz-world-popman` do. It depends on `pz-world`, `pz-kahlua` and `pz-fs`,
  never on `pz-characters`.
  * It owns definitions, `AnimalData` math, the genome, virtual animals,
    zones, tracks, `apop` files, and hutch/trough runtime.
  * It reaches live characters through traits.
* The `IsoAnimal` code that is coupled to the character goes in
  `pz-characters` (`src/character/animal/`), using this crate's types:
  update, states, behaviour, combat, pathing and save.
* The Lua bindings go in `pz-lua-api`.

### (a) IsoAnimal on pz-characters' `GameCharacter` (for the characters thread)

`IsoAnimal extends IsoPlayer`. `pz-characters` already has
`CharacterKind::Animal` (`character/mod.rs`), `is_animal()`, mover kind 4,
`as_player()` returning `Some` for animals, and partial animal branches in
`update.rs` (stats, stress, wake), `fitness.rs`, `combat.rs` and `fire.rs`.

What is needed, verbatim:

1. **Fields.** Add `pub animal: Option<Box<animal::AnimalFields>>` to
   `GameCharacter`. It is `Some` only for `CharacterKind::Animal`, following
   the `zombie: Box<ZombieFields>` pattern but optional. It lives in a new
   `pz-characters/src/character/animal/` module that depends on `pz-animals`
   types. `AnimalFields` holds:
   * **Identity:** `animal_type: String`, `def: Rc<pz_animals::AnimalDefinitions>`
     (`adef`), `breed` (by name), `animal_id: i32`
     (`init`: `Rand.Next(10000)`), `item_id`, `custom_name`,
     `should_be_skeleton`, `road_kill`, `wild`, `invincible`.
   * **`data: AnimalData`** (step 2, in `pz-animals`)
     * Parent: `attachedPlayer` / `attachedTree` (+X/Y).
     * Milk and wool: `milkQty`, `woolQty`, `canHaveMilk`, `maxMilkActual`,
       `lastMilkTimer`.
     * Growth: `weight`, `size`, `originalSize`, `age`,
       `currentStage`/`Nbr`, `lastHourCheck`.
     * Breeding: `pregnant`/`pregnantTime`, `fertilized`/`Time`,
       `lastPregnancyTime`, `lastImpregnateTime`.
     * Eggs: `eggsToday`, `eggTime`, `clutchSize`/`Done`.
     * Hutch and trough: `hutchPosition`, `preferredHutchPosition`,
       `troughPathTimer`, `troughToCheck`, `goingToMom*`, `eatingGrass`,
       `enterHutchTimerAfterDestroy`.
   * **Genome:** `full_genome: JHashMap<AnimalGene>`,
     `genetic_disorder: Vec<String>`.
   * **Stress and needs:** `stress_level`,
     `player_acceptance: Java HashMap<i16, f32>`, `zone_acceptance`,
     `pet_timer`, `milk_removed`, `time_since_flee_from_sound`,
     `last_sound_responded_to`. Hunger and thirst stay in the existing
     `stats`.
   * **Links.** These hold ids or handles, not references.
     * Hutch: `hutch`, `nest_box`, `egg_timer_in_hutch`.
     * Food and water targets: `eat_from_trough`, `drink_from_trough`,
       `eat_from_ground`, `drink_from_river`, `drink_from_puddle`,
       `ignored_trough`, `moving_to_food`/`timer`.
     * Family and handling: `mother`/`mother_id`, `babies`,
       `attach_back_to_mother`/`timer`, `held_by`,
       `lured_by`/`lured_start_timer`/`walk_to_char_luring`.
     * Tree and hook: `attach_back_to_tree_x/y`, `on_hook`/`hook`,
       `attach_back_to_hook_x/y/z`.
     * Designation zones: `d_zone`, `connected_d_zone`, `zone_check_timer`.
   * **Migration and virtual state:** `virtual_id: f64`,
     `migration_group`, `from_meta`, `animal_zone`,
     `move_forward_on_zone`, `last_cell_saved_to_x/y`,
     `time_since_last_update`.
   * **Behaviour:** `behavior: BaseAnimalBehavior` state, `spotted_chr`,
     `alerted`/`alerted_chr`, `atk_target`, `thump_target`/`thump_delay`,
     `fighting_opponent`, `attacked_timer`, `following_wall`,
     `should_follow_wall`, `small_enclosure`, `search_radius`.
   * **Render and sound:** `animal_visual` (`AnimalVisual`),
     `sound_state: JHashMap<AnimalSoundState>`, `next_footstep_sound`,
     `force_next_idle_sound`.
   * Reuse the existing `stats`, `health`, `hours_survived`, `mod_data`,
     `descriptor` (female), `state`/`register_ai_state`, `vars`, `path` and
     `body_damage`. Animals have no `moodles`, `xp`, `nutrition` or
     `fitness` (`IsoGameCharacter:793-800`).
2. **`instanceof IsoPlayer`.** `GameCharacter::is_player()` returns
   `kind == Player`, but Java's `instanceof IsoPlayer` is also true for
   animals.
   * Add `is_iso_player()` (Player or Animal).
   * Audit the roughly 98 `is_player()`, 21 `is_local_player(` and
     25 `CharacterKind::Player` uses.
   * `is_local_player` for an animal is `!GameClient.client`
     (`IsoAnimal.isLocalPlayer`). It does not depend on `in_players`.
3. **Update entry.**
   * `player/update.rs` currently returns early for animals. Replace that
     with `update_animal`: `IsoAnimal.update` (`:384`) → `updateInternal`,
     which runs `behavior.update`, `data.update`, collision width
     `adef.collisionSize*size`, `separate`, tree/zone/mother/sound checks,
     pet timer, stress, luring, thump, `updateLOS` and vehicle collision.
     Then `super.update()`, which is `IsoPlayer.updateInternal1` → only
     `IsoGameCharacter.update()` + `AnimalInstanceManager.update`
     (`IsoPlayer:2015`).
   * Skip `updateInternal2`, all input, mechanics, breathing, temperature,
     weapon, sleeping pills, torch and vehicle endurance.
4. **Overrides to provide**, as `is_animal()` branches or an `AnimalHooks`
   trait implemented in `pz-characters/src/character/animal/`:
   * **Identity and setup:** `getObjectName` returns "Animal";
     `GetAnimSetName` returns `adef.animset`; `initializeStates`
     (`:842`, 12 states; their ids are already in `states/mod.rs:94-105`).
   * **Death:**
     * `OnDeath`, the `onDied` mother cleanup, and
       `shouldBecomeZombieAfterDeath` returning false.
     * `canUseCurrentPoseForCorpse` returns false on a hook; also
       `canRagdoll`, `getCorpseSize`/`Length` and
       `createAnimalFromCorpse`.
   * **Damage:** `hitConsequences` (`data.getHealthLoss`, wild drop-dead
     timer, flee, stress), `setHealth` (invincible), `HitByAnimal`,
     `calcDamage`, `killed`, `Hit(BaseVehicle)` (road kill),
     `testCollideWithVehicles`, `carCrash`.
   * **World add/remove:**
     * `addToWorld` sets the `onhook` variable.
     * `removeFromWorld` (`:1144`) detaches from player, hutch, dZone and
       hook, calls `AnimalManagerWorker.removeFromWorld` and
       `AnimalPopulationManager.addToRecentlyRemoved`.
     * Also `delete`, `remove`, `removeFromUpdateLists`, `unloaded`.
   * **Sight:** `updateLOS` (`:3325`) becomes `behavior.spotted` for
     zombies and non-animal players.
   * **Pathing:**
     * `path_mover.rs` `as_animal()` / `as_animal_mut()` (currently
       "TODO(animal port)") return the character as
       `pz_pathfind::mover::AnimalMover`
       (`should_break_obstacles_during_pathfinding`, `can_climb_fences`,
       `animal_size`).
     * `CanUsePathfindState` returns `isLocalPlayer()`; also
       `getFeelersize`, `canGoThere`, `pathToCharacter`/`Location`/`Trough`
       and `pathFailed`.
   * **Collision:** `IsoMovingObject:298` (stairs, `adef.canClimbStairs`)
     and `:1137` (collidable / `blockMovement`).
   * **Virtualising:** `postupdate:863` virtualises an animal that leaves
     the loaded area while in AnimalZoneState.
   * **Rendering:** `onAnimPlayerCreated` (twist bone `Bip01_Head`),
     `allowsTwist` returns false, `renderlast`, `renderShadow`, `drawRope`,
     `getAttachmentWorldPos`.
   * **IsoPlayer paths that must not run for animals:**
     `RecentlyRemoved` (`IsoPlayer:1519`), bone reparenting (`:1197`),
     human FMOD parameters (`:6481`), corpse-drag sounds (`:6688`), server
     `onKilled` statistics (`:7738`), and `baseVisual` being `HumanVisual`
     with variable callbacks (`:551`).
5. **Save.** `IsoAnimal.save/load` (`:1204-1403`) replaces the IsoPlayer
   format and does not call super.
   * The codec already exists as `pz_world::save::animal::IsoAnimal`.
   * The live animal converts to and from it, and the definitions come from
     `AnimalDefinitions::to_save_definition`.
   * Load calls `init(breed)`, which draws `Rand` (genome, texture, stress,
     wander). That draw must happen even though its results are
     overwritten.
6. **World access.** Add an `AnimalWorld` trait rather than more methods on
   `StateWorld<I>`. It covers troughs, hutches, grass/puddle/river squares,
   trees, `DesignationZoneAnimal`, vehicle animals, butcher hooks and world
   sounds.
7. **Lua class parent.** In `pz-lua-api/bind_character_sim.rs`,
   `ISO_ANIMAL_CLASS` is registered with parent `IsoGameCharacter`. Java's
   parent is `IsoPlayer`.

### (b) Module split for steps 2–7

| Step | Content | Where | Needs from other crates |
|---|---|---|---|
| 2 | `AnimalData` (growth, milk, wool, eggs, pregnancy, `hourGrow`, `updateStatsAway`, hunger/thirst), `IsoAnimal.init`/`initStress`/size/weight, `AnimalVisual`, `IsoAnimal` fields and save conversion | `pz-animals::data` + `pz-characters/character/animal/` | `GameTime` (pz-core `game_time`), `SandboxOptions` (pz-sandbox), `Rand` (`pz_world::GameRand`), `pz_world::save::animal`, `GameCharacter` changes from (a) |
| 3 | AI states (`ai/states/animals/*`: Idle, Walk, PathFind, FollowWall, Eat, Attack, Alerted, ClimbOverFence, HitReaction, Falldown, OnGround, Zone) and `BaseAnimalBehavior` (1504 lines: wander, sit, hutch, eat and drink lookups, follow, flee, `spotted`, attack, `fightAnimal`) | `pz-characters/character/animal/{states,behavior}` | `PathFindBehavior2` + `PolygonalMap2` (pz-pathfind, `AnimalMover`), state machine, `AnimalWorld` hooks, IsoGridSquare flags (pz-world), sandbox/server options |
| 4 | Zones and virtual animals: `AnimalZone`/`AnimalZones`/`AnimalZoneJunction` geometry and junctions, `VirtualAnimal` + `VirtualAnimalState` (Follow/Eat/Sleep), `AnimalTracks`, `spawnAnimalsOnZone`; worldgen `GenAnimalsPath` → `AnimalManagerWorker.allocCell`. `characters/animals/pathfind/*` (mesh A*, nested paths) is only used by the world-map debug UI and comes last. | `pz-animals::{zones, virtual_animal, tracks_runtime}` | zone data (`pz_world::zones`, `animal_zone_save.rs` for map_animals.bin), `StreamEvent::GenAnimalsPath` (`pz_world::streamer`), `pz_world::save::movers` (AnimalTracks), pz-clipper (debug mesh only) |
| 5 | Population: `AnimalPopulationManager`, `AnimalManagerMain` (`fromWorker` materialising virtual animals, `saveRealAnimals`), `AnimalManagerWorker` (cell grid, `loadChunk`, `addAnimal`/`moveAnimal`), `AnimalCell`/`AnimalChunk`, the `apop/apop_<cx>_<cy>.bin` codec (int 249; 1024 chunks of short n + VirtualAnimals + short n + tracks), and the online-id map of `AnimalInstanceManager` (also used in single player) | `pz-animals::population` (all on the main thread; the original's "worker" is synchronous) | `SaveGameHooks::save_animals` (`pz-lua-api/world_save.rs:207`), `StreamEvent::HandedOff` (`addChunkToWorld`) / `Unloaded` (`removeChunkFromWorld`) / `HotSave` (`saveToBufferMap`), `IsoWorld.init` (`init(metaGrid)` after map_animals), `IsoWorld` update (`updateVirtualAnimals`, `HutchManager.updateAll`), resets (`Core:4122`, `IngameState:1008`), a character factory trait to create `IsoAnimal`s |
| 6 | `IsoHutch` runtime (`update`: dirt, animals inside, eggs and hatching, exit timer, `doMeta`, `HutchManager`), `IsoFeedingTrough` runtime (rain water, fill-level overlays, linked animals, `addToProcessIsoObject`), `IsoAnimalTrack` (`glow`), `DesignationZoneAnimal` runtime | `pz-animals::{hutch, trough}` + world object hooks | saved fields already in `pz_world` (`objects.rs` HutchMaster/NestBox/trough/track, `object_io.rs`, `meta_save.rs`), sprite manager, IsoRegions, climate/rain |
| 7 | Lua: the `IsoAnimal` methods not on IsoPlayer (195), `AnimalData` (99), `BaseAnimalBehavior`, `VirtualAnimal`, `AnimalTracks`, `AnimalChunk`, definitions/breed/gene/allele/genome/parts classes, `IsoHutch`/`NestBox`/`IsoFeedingTrough`/`IsoAnimalTrack`, `DesignationZoneAnimal`, `AnimalInventoryItem`, and globals (`getAnimalChunk`, `getHutch`, `addAnimal`, `removeAnimal`, `getAnimal`, `getAllAnimalsDefinitions`, `send*Animal*`) | `pz-lua-api/bind_animals.rs` | `reference/lua-exposure.jsonl.gz` for the exact surface |

### (c) Oracle approach per step

All oracles are scratch JVM harnesses: ecj or `javac --release 21` with
reflection only, the bundled JRE, `RandStandard.INSTANCE.init()` plus a
seeded `Random`, a scratch HOME, `-Duser.home` and
`-Ddeployment.user.cachedir`. They are never committed.

* **Step 2.**
  * Build an `IsoAnimal` with `Unsafe.allocateInstance` and set `adef`,
    `type`, the descriptor and a real `AnimalData(animal, breed)`.
  * Trace `initStress`, `AnimalData.init`/`initSize`/`initWeight`,
    `hourGrow`, `growUp`, `checkEggs`, `updateMilk`/`getMilkInc`/`getWoolInc`,
    `checkPregnancy`, `updateStatsAway(hours)` and
    `checkKilledByMetaPredator`.
  * Use a fake `GameTime` (set the instance's multiplier and calendar by
    reflection), then diff per-hour traces.
  * Round-trip `IsoAnimal.save/load` byte for byte, including the `Rand`
    consumed by `init` during load.
* **Step 3.**
  * Oracle the pure helpers: `pickRandomWanderInterval`, `checkSit`,
    acceptance and stress math, `calcDamage`, `getHealthLoss`,
    `canEatThis`.
  * For whole states, follow the `pz-world-states` `mover_parity` pattern:
    a dumped region, a fake clock and seeded `Rand`. Fall back to in-game
    Lua logging traces from a scratch-HOME game run.
  * Animation-event-driven transitions cannot be run headless.
* **Step 4.**
  * `AnimalZone` geometry is pure and can be oracled directly:
    `getPointOnPolyline`, `getClosestPointOnPolyline`,
    `getJunctionsBetween`, `getDistanceOfPointFromStart`,
    `addJunctionsWithOtherZone`.
  * `VirtualAnimalState` stepping and `spawnAnimalsOnZone` run with a fake
    world-age clock and seeded `Rand` on real map zones, reusing the
    world-load oracle setup.
* **Step 5.**
  * Codec: parse real `apop_*.bin` from a copied save (never `~/Zomboid`)
    and check the re-save is byte-identical.
  * Semantics: run `AnimalCell.load` + `save` in the JVM with the meta-grid
    `animalZoneHandler` loaded, with seeded `Rand` on both sides.
  * The chunk add/remove sequence can be replayed from `StreamEvent` logs.
* **Step 6.** `IsoHutch.update`/`doMeta` on a reflected hutch with a fake
  hour and seeded `Rand`. Trough overlays need the sprite manager, which is
  available in the world-load oracle.
* **Step 7.** Compare the bound method names and arities against
  `lua-exposure.jsonl.gz`. Check behaviour through the parity of the
  underlying methods above.
* **Cannot be oracled headless:** `AnimalVisual` and textures, `render*`,
  `drawRope`, `getAttachmentWorldPos` (ModelScript + AnimationPlayer),
  FMOD sounds (`AnimalSoundState`, `playBreedSound`), ragdoll, and network
  sync. These are checked visually or by audio replay.
