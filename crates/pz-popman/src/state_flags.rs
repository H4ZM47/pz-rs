//! `zombie.popman.ZombieStateFlag` / `ZombieStateFlags`: the state bits a
//! zombie keeps while it is not real.

/// `ZombieStateFlag`, in ordinal order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ZombieStateFlag {
    Initialized,
    Crawling,
    CanWalk,
    FakeDead,
    CanCrawlUnderVehicle,
    ReanimatedForGrappleOnly,
}

impl ZombieStateFlag {
    /// `ZombieStateFlag.values()`.
    pub const ALL: [ZombieStateFlag; 6] = [
        ZombieStateFlag::Initialized,
        ZombieStateFlag::Crawling,
        ZombieStateFlag::CanWalk,
        ZombieStateFlag::FakeDead,
        ZombieStateFlag::CanCrawlUnderVehicle,
        ZombieStateFlag::ReanimatedForGrappleOnly,
    ];

    /// The `flag` field.
    pub fn flag(self) -> i32 {
        1 << self as i32
    }

    /// `name()`.
    pub fn name(self) -> &'static str {
        match self {
            ZombieStateFlag::Initialized => "Initialized",
            ZombieStateFlag::Crawling => "Crawling",
            ZombieStateFlag::CanWalk => "CanWalk",
            ZombieStateFlag::FakeDead => "FakeDead",
            ZombieStateFlag::CanCrawlUnderVehicle => "CanCrawlUnderVehicle",
            ZombieStateFlag::ReanimatedForGrappleOnly => "ReanimatedForGrappleOnly",
        }
    }
}

/// What `ZombieStateFlags.intFromZombie` reads from an `IsoZombie`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ZombieStateSource {
    pub crawling: bool,
    pub can_walk: bool,
    pub fake_dead: bool,
    pub can_crawl_under_vehicle: bool,
    pub reanimated_for_grapple_only: bool,
}

/// `ZombieStateFlags`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ZombieStateFlags {
    flags: i32,
}

impl ZombieStateFlags {
    /// `fromInt`.
    pub fn from_int(flags: i32) -> ZombieStateFlags {
        ZombieStateFlags { flags }
    }

    /// `new ZombieStateFlags(flags...)`.
    pub fn from_flags(flags: &[ZombieStateFlag]) -> ZombieStateFlags {
        let mut s = ZombieStateFlags::default();
        for &f in flags {
            s.set_flag(f);
        }
        s
    }

    /// `intFromZombie`.
    pub fn int_from_zombie(z: &ZombieStateSource) -> i32 {
        let mut state = 0;
        state = Self::set(state, ZombieStateFlag::Initialized);
        state = Self::set_to(state, ZombieStateFlag::Crawling, z.crawling);
        state = Self::set_to(state, ZombieStateFlag::CanWalk, z.can_walk);
        state = Self::set_to(state, ZombieStateFlag::FakeDead, z.fake_dead);
        state = Self::set_to(state, ZombieStateFlag::CanCrawlUnderVehicle, z.can_crawl_under_vehicle);
        Self::set_to(state, ZombieStateFlag::ReanimatedForGrappleOnly, z.reanimated_for_grapple_only)
    }

    /// `fromZombie`.
    pub fn from_zombie(z: &ZombieStateSource) -> ZombieStateFlags {
        Self::from_int(Self::int_from_zombie(z))
    }

    /// Static `setFlag(state, flag)`.
    pub fn set(state: i32, flag: ZombieStateFlag) -> i32 {
        state | flag.flag()
    }

    /// Static `clearFlag(state, flag)`.
    pub fn clear(state: i32, flag: ZombieStateFlag) -> i32 {
        state & !flag.flag()
    }

    /// Static `setFlag(state, flag, isTrue)`.
    pub fn set_to(state: i32, flag: ZombieStateFlag, on: bool) -> i32 {
        if on {
            Self::set(state, flag)
        } else {
            Self::clear(state, flag)
        }
    }

    /// Static `checkFlag(state, flag)`.
    pub fn check(state: i32, flag: ZombieStateFlag) -> bool {
        state & flag.flag() != 0
    }

    pub fn set_flag(&mut self, flag: ZombieStateFlag) {
        self.flags = Self::set(self.flags, flag);
    }

    pub fn clear_flag(&mut self, flag: ZombieStateFlag) {
        self.flags = Self::clear(self.flags, flag);
    }

    pub fn set_flag_to(&mut self, flag: ZombieStateFlag, on: bool) {
        self.flags = Self::set_to(self.flags, flag, on);
    }

    pub fn check_flag(&self, flag: ZombieStateFlag) -> bool {
        Self::check(self.flags, flag)
    }

    /// `asInt`.
    pub fn as_int(&self) -> i32 {
        self.flags
    }

    pub fn is_initialized(&self) -> bool {
        self.check_flag(ZombieStateFlag::Initialized)
    }

    pub fn is_crawling(&self) -> bool {
        self.check_flag(ZombieStateFlag::Crawling)
    }

    pub fn is_can_walk(&self) -> bool {
        self.check_flag(ZombieStateFlag::CanWalk)
    }

    pub fn is_fake_dead(&self) -> bool {
        self.check_flag(ZombieStateFlag::FakeDead)
    }

    pub fn is_can_crawl_under_vehicle(&self) -> bool {
        self.check_flag(ZombieStateFlag::CanCrawlUnderVehicle)
    }

    pub fn is_reanimated_for_grapple_only(&self) -> bool {
        self.check_flag(ZombieStateFlag::ReanimatedForGrappleOnly)
    }

    /// `toArray`.
    pub fn to_array(&self) -> Vec<ZombieStateFlag> {
        ZombieStateFlag::ALL.into_iter().filter(|&f| self.check_flag(f)).collect()
    }

    /// `toString`.
    pub fn to_java_string(&self) -> String {
        let names: Vec<&str> = self.to_array().iter().map(|f| f.name()).collect();
        format!("zombie.popman.ZombieStateFlags{{ {{ {} }}}}", names.join(", "))
    }

    /// `intToString`.
    pub fn int_to_string(state: i32) -> String {
        Self::from_int(state).to_java_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_round_trip() {
        let src = ZombieStateSource { crawling: true, fake_dead: true, ..Default::default() };
        let v = ZombieStateFlags::int_from_zombie(&src);
        assert_eq!(v, 1 | 2 | 8);
        let f = ZombieStateFlags::from_int(v);
        assert!(f.is_initialized() && f.is_crawling() && f.is_fake_dead() && !f.is_can_walk());
        assert_eq!(f.to_array(), vec![ZombieStateFlag::Initialized, ZombieStateFlag::Crawling, ZombieStateFlag::FakeDead]);
        assert_eq!(
            ZombieStateFlags::int_to_string(v),
            "zombie.popman.ZombieStateFlags{ { Initialized, Crawling, FakeDead }}"
        );
        assert_eq!(ZombieStateFlags::clear(v, ZombieStateFlag::Crawling), 9);
    }
}
