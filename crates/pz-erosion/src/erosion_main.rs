//! `zombie.erosion.ErosionMain`: configuration, the erosion clock and the
//! per-chunk / per-square entry points called while chunks load.

use std::collections::HashMap;
use std::path::PathBuf;

use pz_world::{GameRand, IsoChunk, IsoSpriteManager};

use crate::bits::pack_float_unit_to_byte;
use crate::category::Ctx;
use crate::config::ErosionConfig;
use crate::data::{ChunkData, SquareData, SquareRand};
use crate::ice_queen::ErosionIceQueen;
use crate::noise::Noise2D;
use crate::rand::world_gen_random;
use crate::regions::ErosionRegions;
use crate::season::ErosionSeason;
use crate::tables::ErosionTables;
use crate::world_host::{ChunkHost, WorldView};
use pz_core::calendar::JavaZone;

/// Engine state `ErosionMain` reads (`SandboxOptions`, `GameTime`,
/// `GameServer`/`GameClient`, `Core`, `ZomboidFileSystem`).
#[derive(Clone, Debug)]
pub struct ErosionSettings {
    /// `SandboxOptions.getErosionSpeed()` (1..=5).
    pub erosion_speed: i32,
    /// `SandboxOptions.erosionDays`.
    pub erosion_days: i32,
    /// `SandboxOptions.timeSinceApo`.
    pub time_since_apo: i32,
    /// `SandboxOptions.getTemperatureModifier()` (1..=5).
    pub temperature_modifier: i32,
    /// `SandboxOptions.dayNightCycle`.
    pub day_night_cycle: i32,
    /// `GameTime.getDay()` (0-based), `getMonth()` (0-based), `getYear()`.
    pub day: i32,
    pub month: i32,
    pub year: i32,
    pub server: bool,
    pub client: bool,
    /// `ErosionData.staticRand`.
    pub static_rand: bool,
    /// `Core.debug` (the client writes the config every tick in debug).
    pub core_debug: bool,
    /// `Core.getInstance().isNoSave()`.
    pub no_save: bool,
    /// `WorldGenParams.seed` (the seed string's `hashCode()`).
    pub world_seed: i32,
    /// `ZomboidFileSystem.getFileNameInCurrentSave("erosion.ini")`.
    pub save_config: Option<PathBuf>,
    /// `ZomboidFileSystem.getCacheDir()/erosion.ini`.
    pub cache_config: Option<PathBuf>,
    /// `ZomboidFileSystem.getMediaFile("data/erosion.ini")`.
    pub media_config: Option<PathBuf>,
}

impl Default for ErosionSettings {
    /// Single-player defaults (sandbox defaults, July 9th 1993).
    fn default() -> Self {
        ErosionSettings {
            erosion_speed: 3,
            erosion_days: 0,
            time_since_apo: 1,
            temperature_modifier: 3,
            day_night_cycle: 1,
            day: 8,
            month: 6,
            year: 1993,
            server: false,
            client: false,
            static_rand: false,
            core_debug: false,
            no_save: false,
            world_seed: 0,
            save_config: None,
            cache_config: None,
            media_config: None,
        }
    }
}

/// `LuaEventManager.triggerEvent("OnInitSeasons", season)`.
pub type SeasonHook = Box<dyn FnMut(&mut ErosionSeason)>;

/// `ErosionMain`.
pub struct ErosionMain {
    pub tables: ErosionTables,
    pub settings: ErosionSettings,
    pub cfg: ErosionConfig,
    pub debug: bool,
    pub ice_queen: ErosionIceQueen,
    is_snow: bool,
    pub noise_main: Noise2D,
    pub noise_moisture: Noise2D,
    pub noise_minerals: Noise2D,
    pub noise_kudzu: Noise2D,
    pub world: ErosionRegions,
    pub season: ErosionSeason,
    pub tick_unit: i32,
    pub ticks: i32,
    pub e_ticks: i32,
    pub day: i32,
    pub month: i32,
    pub year: i32,
    pub epoch: i32,
    snow_frac: i32,
    snow_frac_yesterday: i32,
    /// In-memory square state of loaded chunks (fields the save format does
    /// not keep), by world square. [`ErosionChunkHook`] drops a chunk's
    /// entries when `IsoCell::unload_chunk` reports it.
    squares: HashMap<(i32, i32, i32), SquareData>,
    /// Called with the season after it is set up (`OnInitSeasons`).
    pub on_init_seasons: Option<SeasonHook>,
}

impl ErosionMain {
    /// `new ErosionMain(manager, debug)` (which runs `start()`).
    pub fn new(
        tables: ErosionTables,
        settings: ErosionSettings,
        sprites: &mut IsoSpriteManager,
        zone: Option<JavaZone>,
        on_init_seasons: Option<SeasonHook>,
    ) -> Self {
        let perm = tables.noise_perm.clone();
        let mut m = ErosionMain {
            cfg: ErosionConfig::new(&tables),
            debug: true,
            ice_queen: ErosionIceQueen::default(),
            is_snow: false,
            noise_main: Noise2D::new(perm.clone()),
            noise_moisture: Noise2D::new(perm.clone()),
            noise_minerals: Noise2D::new(perm.clone()),
            noise_kudzu: Noise2D::new(perm.clone()),
            world: ErosionRegions::default(),
            season: ErosionSeason::new(perm, tables.season_names.clone(), zone),
            tick_unit: 144,
            ticks: 0,
            e_ticks: 0,
            day: 0,
            month: 0,
            year: 0,
            epoch: 0,
            snow_frac: 0,
            snow_frac_yesterday: 0,
            squares: HashMap::new(),
            on_init_seasons,
            tables,
            settings,
        };
        m.start(sprites);
        m
    }

    pub fn get_config(&self) -> &ErosionConfig {
        &self.cfg
    }

    pub fn get_seasons(&self) -> &ErosionSeason {
        &self.season
    }

    pub fn get_etick(&self) -> i32 {
        self.e_ticks
    }

    pub fn get_snow_fraction(&self) -> i32 {
        self.snow_frac
    }

    pub fn get_snow_fraction_yesterday(&self) -> i32 {
        self.snow_frac_yesterday
    }

    pub fn is_snow(&self) -> bool {
        self.is_snow
    }

    /// `snowCheck()` (empty upstream).
    pub fn snow_check(&mut self) {}

    /// `initConfig()`. A config is never written to the user's folders by
    /// this crate; only `cache_config` (when set) receives a copy of the
    /// media config, as upstream.
    fn init_config(&mut self) -> bool {
        self.cfg = ErosionConfig::new(&self.tables);
        if let Some(p) = &self.settings.save_config {
            if p.exists() {
                if self.cfg.read_file(p) {
                    return true;
                }
                self.cfg = ErosionConfig::new(&self.tables);
            }
        }
        if let Some(cache) = &self.settings.cache_config {
            if !cache.exists() && !self.settings.no_save {
                if let Some(src) = self.settings.media_config.as_ref().filter(|s| s.exists()) {
                    if let Err(e) = std::fs::copy(src, cache) {
                        eprintln!("erosion: {e}");
                    }
                }
            }
            if cache.exists() && !self.cfg.read_file(cache) {
                self.cfg = ErosionConfig::new(&self.tables);
            }
        }
        let t = &mut self.cfg.time;
        match self.settings.erosion_speed {
            1 => t.tickunit /= 5,
            2 => t.tickunit /= 2,
            4 => t.tickunit *= 2,
            5 => t.tickunit *= 5,
            _ => {}
        }
        let days_till_100 = t.tickunit.wrapping_mul(100) as f32 / 144.0;
        let days_since_apo = ((self.settings.time_since_apo - 1) * 30) as f32;
        t.eticks = ((1.0f32.min(days_since_apo / days_till_100) * 100.0) as f64).floor() as i32;
        let erosion_days = self.settings.erosion_days;
        if erosion_days > 0 {
            t.tickunit = 144;
            t.eticks =
                ((1.0f32.min(days_since_apo / erosion_days as f32) * 100.0) as f64).floor() as i32;
        }
        true
    }

    /// `start()`.
    pub fn start(&mut self, sprites: &mut IsoSpriteManager) {
        if !self.init_config() {
            return;
        }
        self.tick_unit = self.cfg.time.tickunit;
        self.ticks = self.cfg.time.ticks;
        self.e_ticks = self.cfg.time.eticks;
        self.month = self.settings.month;
        self.year = self.settings.year;
        self.day = self.settings.day;
        self.debug = !self.settings.server && self.cfg.debug.enabled;
        let s = &self.cfg.seeds;
        self.noise_main.reset();
        self.noise_main.add_layer(s.seed_main_0, 0.5, 3.0);
        self.noise_main.add_layer(s.seed_main_1, 2.0, 5.0);
        self.noise_main.add_layer(s.seed_main_2, 5.0, 8.0);
        self.noise_moisture.reset();
        self.noise_moisture.add_layer(s.seed_moisture_0, 2.0, 3.0);
        self.noise_moisture.add_layer(s.seed_moisture_1, 1.6, 5.0);
        self.noise_moisture.add_layer(s.seed_moisture_2, 0.6, 8.0);
        self.noise_minerals.reset();
        self.noise_minerals.add_layer(s.seed_minerals_0, 2.0, 3.0);
        self.noise_minerals.add_layer(s.seed_minerals_1, 1.6, 5.0);
        self.noise_minerals.add_layer(s.seed_minerals_2, 0.6, 8.0);
        self.noise_kudzu.reset();
        self.noise_kudzu.add_layer(s.seed_kudzu_0, 6.0, 3.0);
        self.noise_kudzu.add_layer(s.seed_kudzu_1, 3.0, 5.0);
        self.noise_kudzu.add_layer(s.seed_kudzu_2, 0.5, 8.0);
        let sc = self.cfg.season.clone();
        let (mut temp_min, mut temp_max) = (sc.temp_min, sc.temp_max);
        match self.settings.temperature_modifier {
            1 => {
                temp_min -= 10;
                temp_max -= 10;
            }
            2 => {
                temp_min -= 5;
                temp_max -= 5;
            }
            4 => {
                temp_min = (temp_min as f64 + 7.5) as i32;
                temp_max += 4;
            }
            5 => {
                temp_min += 15;
                temp_max += 8;
            }
            _ => {}
        }
        self.season = ErosionSeason::new(
            self.tables.noise_perm.clone(),
            self.tables.season_names.clone(),
            Some(self.season_zone()),
        );
        self.season.day_night_cycle = self.settings.day_night_cycle;
        self.season.init(
            sc.lat,
            temp_max,
            temp_min,
            sc.temp_diff,
            sc.season_lag,
            sc.noon,
            sc.seed_a,
            sc.seed_b,
            sc.seed_c,
        );
        self.season.set_rain(sc.rain);
        self.season.set_day(self.day, self.month, self.year);
        if let Some(f) = self.on_init_seasons.as_mut() {
            f(&mut self.season);
        }
        self.ice_queen = ErosionIceQueen::new(sprites);
        let (world, ice) = ErosionRegions::init(&self.tables);
        self.world = world;
        for (a, b) in ice {
            self.ice_queen.add_sprite(sprites, &a, &b);
        }
        self.snow_check();
    }

    fn season_zone(&self) -> JavaZone {
        self.season.zone()
    }

    /// `mainTimer()` (`EveryTenMinutes`), with the current game date.
    pub fn main_timer(&mut self, day: i32, month: i32, year: i32) {
        if self.settings.client {
            if self.settings.core_debug {
                self.write_config();
            }
            return;
        }
        let erosion_days = self.settings.erosion_days;
        if self.debug {
            self.e_ticks = self.e_ticks.wrapping_add(1);
        } else if erosion_days < 0 {
            self.e_ticks = 0;
        } else if erosion_days > 0 {
            self.ticks += 1;
            self.e_ticks = (self.ticks as f32 / 144.0 / erosion_days as f32 * 100.0) as i32;
        } else {
            self.ticks += 1;
            if self.ticks >= self.tick_unit {
                self.ticks = 0;
                self.e_ticks = self.e_ticks.wrapping_add(1);
            }
        }
        if self.e_ticks < 0 {
            self.e_ticks = i32::MAX;
        }
        if day != self.day || month != self.month || year != self.year {
            self.month = month;
            self.year = year;
            self.day = day;
            self.epoch += 1;
            self.season.set_day(self.day, self.month, self.year);
            self.snow_check();
        }
        self.cfg.time.ticks = self.ticks;
        self.cfg.time.eticks = self.e_ticks;
        self.cfg.time.epoch = self.epoch;
        self.write_config();
    }

    fn write_config(&self) {
        if let Some(p) = &self.settings.save_config {
            self.cfg.write_file(p, self.settings.no_save);
        }
    }

    /// `sendState(bb)` (server only).
    pub fn send_state(&self, time_of_day: f32, out: &mut Vec<u8>) {
        if !self.settings.server {
            return;
        }
        out.extend_from_slice(&self.e_ticks.to_be_bytes());
        out.extend_from_slice(&self.ticks.to_be_bytes());
        out.extend_from_slice(&self.epoch.to_be_bytes());
        out.push(self.snow_frac as u8);
        out.push(self.snow_frac_yesterday as u8);
        out.extend_from_slice(&time_of_day.to_bits().to_be_bytes());
    }

    /// `receiveState(bb)` (client only). Returns whether the map needs
    /// `updateMapNow` (the caller runs [`ErosionMain::update_chunk_now`] on
    /// its loaded chunks).
    pub fn receive_state(&mut self, data: &[u8], day: i32, month: i32, year: i32) -> bool {
        if !self.settings.client || data.len() < 18 {
            return false;
        }
        let int = |i: usize| i32::from_be_bytes(data[i..i + 4].try_into().unwrap());
        let (old_ticks, old_epoch) = (self.e_ticks, self.epoch);
        self.e_ticks = int(0);
        self.ticks = int(4);
        self.epoch = int(8);
        self.cfg.time.ticks = self.ticks;
        self.cfg.time.eticks = self.e_ticks;
        self.cfg.time.epoch = self.epoch;
        if day != self.day || month != self.month || year != self.year {
            self.month = month;
            self.year = year;
            self.day = day;
            self.season.set_day(self.day, self.month, self.year);
        }
        old_ticks != self.e_ticks || old_epoch != self.epoch
    }

    /// `initChunk(chunk, chunkModData)`.
    pub fn init_chunk(&self, wx: i32, wy: i32, c: &mut ChunkData) {
        c.set(wx, wy);
        let nx = c.x as f32 / 5.0;
        let ny = c.y as f32 / 5.0;
        let moisture = self.noise_moisture.layered_noise(nx, ny);
        let minerals = self.noise_minerals.layered_noise(nx, ny);
        let moi = if moisture < 1.0 {
            ((moisture * 10.0) as f64).floor() as usize
        } else {
            9
        };
        let min = if minerals < 1.0 {
            ((minerals * 10.0) as f64).floor() as usize
        } else {
            9
        };
        c.init = true;
        c.e_tick_stamp = -1;
        c.epoch = -1;
        c.moisture = moisture;
        c.minerals = minerals;
        c.soil = self.tables.soil_table[moi][min] - 1;
    }

    /// `initGridSquare(square, erosionModData)` for the square at world
    /// `(x, y)`.
    pub fn init_grid_square(
        &self,
        x: i32,
        y: i32,
        sd: &mut SquareData,
        chunk: &ChunkData,
        global: &mut dyn GameRand,
    ) {
        let noise = self
            .noise_main
            .layered_noise(x as f32 / 10.0, y as f32 / 10.0);
        sd.noise_main_byte = pack_float_unit_to_byte(noise);
        sd.noise_main = noise;
        sd.noise_main_int = ((sd.noise_main * 100.0) as f64).floor() as i32;
        sd.noise_kudzu = self
            .noise_kudzu
            .layered_noise(x as f32 / 10.0, y as f32 / 10.0);
        sd.soil = chunk.soil;
        sd.rand = Some(world_gen_random(self.settings.world_seed, x, y, 0));
        let mut r = self.square_rand(global);
        let magic = sd.rand(x, y, 100, &mut r) as f32 / 100.0;
        sd.magic_num_byte = pack_float_unit_to_byte(magic);
        sd.magic_num = magic;
        sd.regions.clear();
        sd.init = true;
    }

    fn square_rand<'a>(&'a self, global: &'a mut dyn GameRand) -> SquareRand<'a> {
        SquareRand {
            use_static: self.settings.static_rand || self.settings.server || self.settings.client,
            table: &self.tables.square_rands,
            global,
        }
    }

    /// The square's in-memory erosion state: kept from earlier calls, or
    /// rebuilt from its saved record.
    fn take_square(&mut self, chunk: &mut IsoChunk, sq: (i32, i32, i32)) -> SquareData {
        if let Some(d) = self.squares.remove(&sq) {
            return d;
        }
        let (lx, ly) = (sq.0 - chunk.wx * 8, sq.1 - chunk.wy * 8);
        chunk
            .get_grid_square(lx, ly, sq.2)
            .and_then(|s| s.erosion.as_ref())
            .map(SquareData::from_saved)
            .unwrap_or_default()
    }

    fn put_square(&mut self, chunk: &mut IsoChunk, sq: (i32, i32, i32), d: SquareData) {
        let (lx, ly) = (sq.0 - chunk.wx * 8, sq.1 - chunk.wy * 8);
        if let Some(s) = chunk.get_grid_square_mut(lx, ly, sq.2) {
            s.erosion = Some(d.to_saved());
        }
        self.squares.insert(sq, d);
    }

    fn chunk_data(chunk: &IsoChunk) -> ChunkData {
        ChunkData::from_saved(chunk.erosion.as_ref(), chunk.wx, chunk.wy)
    }

    /// `loadGridsquare(square)` for chunk-local `(x, y, z)`.
    #[allow(clippy::too_many_arguments)]
    pub fn load_grid_square(
        &mut self,
        chunk: &mut IsoChunk,
        x: i32,
        y: i32,
        z: i32,
        sprites: &mut IsoSpriteManager,
        world: &WorldView<'_>,
        global: &mut dyn GameRand,
        logs_per_size: Option<&[i32]>,
    ) {
        if z != 0 || chunk.get_grid_square(x, y, z).is_none() {
            return;
        }
        let pos = (chunk.wx * 8 + x, chunk.wy * 8 + y, z);
        // getChunk(square)
        let mut cd = Self::chunk_data(chunk);
        if !cd.init {
            self.init_chunk(chunk.wx, chunk.wy, &mut cd);
            chunk.erosion = Some(cd.to_saved());
        }
        let mut sd = self.take_square(chunk, pos);
        let use_static = self.settings.static_rand || self.settings.server || self.settings.client;
        let (e_ticks, epoch) = (self.e_ticks, self.epoch);
        {
            let mut host = ChunkHost {
                chunk,
                sprites,
                world,
                logs_per_size,
            };
            if !sd.init {
                self.init_grid_square(pos.0, pos.1, &mut sd, &cd, global);
                let mut ctx = Ctx {
                    season: &self.season,
                    rand: SquareRand {
                        use_static,
                        table: &self.tables.square_rands,
                        global,
                    },
                };
                self.world
                    .validate_spawn(&mut host, pos, &mut sd, &cd, &mut ctx);
            }
            if !sd.do_nothing && !(cd.e_tick_stamp >= e_ticks && cd.epoch == epoch) {
                let ctx = Ctx {
                    season: &self.season,
                    rand: SquareRand {
                        use_static,
                        table: &self.tables.square_rands,
                        global,
                    },
                };
                self.world.update(&mut host, pos, &mut sd, e_ticks, &ctx);
            }
        }
        self.put_square(chunk, pos, sd);
    }

    /// `loadChunk(chunk)` (`ChunkLoaded`).
    pub fn chunk_loaded(&mut self, chunk: &mut IsoChunk) {
        let mut cd = Self::chunk_data(chunk);
        if !cd.init {
            self.init_chunk(chunk.wx, chunk.wy, &mut cd);
        }
        cd.e_tick_stamp = self.e_ticks;
        cd.epoch = self.epoch;
        chunk.erosion = Some(cd.to_saved());
    }

    /// The per-chunk part of `updateMapNow()` / the server's `mainTimer`
    /// loop: re-run every z=0 square of a chunk whose stamps are stale.
    #[allow(clippy::too_many_arguments)]
    pub fn update_chunk_now(
        &mut self,
        chunk: &mut IsoChunk,
        sprites: &mut IsoSpriteManager,
        world: &WorldView<'_>,
        global: &mut dyn GameRand,
        logs_per_size: Option<&[i32]>,
    ) {
        let cd = Self::chunk_data(chunk);
        if cd.e_tick_stamp == self.e_ticks && cd.epoch == self.epoch {
            return;
        }
        for yy in 0..8 {
            for xx in 0..8 {
                if chunk.get_grid_square(xx, yy, 0).is_some() {
                    self.load_grid_square(chunk, xx, yy, 0, sprites, world, global, logs_per_size);
                }
            }
        }
        let mut cd = Self::chunk_data(chunk);
        cd.e_tick_stamp = self.e_ticks;
        cd.epoch = self.epoch;
        chunk.erosion = Some(cd.to_saved());
    }

    /// Forgets the in-memory state of a chunk's squares (the chunk was
    /// unloaded; its saved records stay in the chunk).
    pub fn chunk_unloaded(&mut self, wx: i32, wy: i32) {
        self.squares
            .retain(|&(x, y, _), _| x.div_euclid(8) != wx || y.div_euclid(8) != wy);
    }

    /// The in-memory state of a loaded square, if erosion has seen it.
    pub fn square_data(&self, x: i32, y: i32, z: i32) -> Option<&SquareData> {
        self.squares.get(&(x, y, z))
    }

    /// `IsoGridSquare.getErosionData().reset()` for a loaded square.
    pub fn reset_square(&mut self, chunk: &mut IsoChunk, x: i32, y: i32, z: i32) {
        let pos = (chunk.wx * 8 + x, chunk.wy * 8 + y, z);
        let mut d = self.take_square(chunk, pos);
        d.reset();
        self.put_square(chunk, pos, d);
    }

    /// `IsoGridSquare.disableErosion()`.
    pub fn disable_erosion(&mut self, chunk: &mut IsoChunk, x: i32, y: i32, z: i32) {
        let pos = (chunk.wx * 8 + x, chunk.wy * 8 + y, z);
        let mut d = self.take_square(chunk, pos);
        d.do_nothing = true;
        self.put_square(chunk, pos, d);
    }

    /// `IsoGridSquare.removeErosionObject(type)`.
    pub fn remove_erosion_object(
        &mut self,
        chunk: &mut IsoChunk,
        x: i32,
        y: i32,
        z: i32,
        kind: &str,
    ) {
        if kind != "WallVines" {
            return;
        }
        let pos = (chunk.wx * 8 + x, chunk.wy * 8 + y, z);
        let mut d = self.take_square(chunk, pos);
        d.clear_cat_mod_data(2, 0);
        self.put_square(chunk, pos, d);
    }
}

/// [`pz_world::save::chunk_file::ChunkLoadHook`] running erosion on chunks
/// loaded from a save.
pub struct ErosionChunkHook<'a> {
    pub main: &'a mut ErosionMain,
    pub sprites: &'a mut IsoSpriteManager,
    pub world: WorldView<'a>,
    /// `Rand`, for squares loaded without their `RandLocation`.
    pub rand: &'a mut dyn GameRand,
    pub logs_per_size: Option<&'a [i32]>,
}

impl pz_world::save::chunk_file::ChunkLoadHook for ErosionChunkHook<'_> {
    fn load_grid_square(&mut self, chunk: &mut IsoChunk, x: i32, y: i32, z: i32) {
        self.main.load_grid_square(
            chunk,
            x,
            y,
            z,
            self.sprites,
            &self.world,
            self.rand,
            self.logs_per_size,
        );
    }

    fn chunk_loaded(&mut self, chunk: &mut IsoChunk) {
        self.main.chunk_loaded(chunk);
    }

    fn chunk_unloaded(&mut self, wx: i32, wy: i32) {
        self.main.chunk_unloaded(wx, wy);
    }
}
