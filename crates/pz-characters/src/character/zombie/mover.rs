//! `pz_pathfind::mover::ZombieMover` for zombies (`IsoZombie` as
//! `PathFindBehavior2` and the path request see it).

use pz_pathfind::mover::ZombieMover;

use super::GameCharacter;

impl<I: 'static> ZombieMover for GameCharacter<I> {
    fn is_crawling(&self) -> bool {
        self.vars.crawling
    }
    fn is_can_walk(&self) -> bool {
        self.zombie.can_walk
    }
    fn is_can_crawl_under_vehicle(&self) -> bool {
        self.zombie.can_crawl_under_vehicle
    }
    fn has_target(&self) -> bool {
        self.vars.target.is_some()
    }
    /// `isMovingToPlayerSound()`: the behaviour's goal is a sound while
    /// path finding or walking (`last_heard_sound` is set exactly when the
    /// zombie paths to a sound and cleared outside those states).
    fn is_moving_to_player_sound(&self) -> bool {
        let goal_sound = self.zombie.last_heard_sound.0 != -1;
        self.is_moving_to_player_sound_with(goal_sound)
    }
    fn is_remote_zombie(&self) -> bool {
        self.zombie.remote
    }
    fn allow_repath_delay(&self) -> f32 {
        self.zombie.allow_repath_delay
    }
    fn set_allow_repath_delay(&mut self, v: f32) {
        self.zombie.allow_repath_delay = v;
    }
    fn set_running(&mut self, running: bool) {
        self.zombie.running = running;
    }
    fn is_ghost(&self) -> bool {
        self.zombie.ghost
    }
    fn set_ghost(&mut self, ghost: bool) {
        self.zombie.ghost = ghost;
    }
    fn get_down_is_near_start_xy(&self) -> bool {
        crate::character::states::zombie::get_down_is_near_start_xy(self)
    }
    fn get_down_set_params(&mut self) {
        // `Rand.Next(60.0f, 150.0f)` on the standard generator.
        let wait = pz_core::random::next_float(60.0, 150.0);
        crate::character::states::zombie::GET_DOWN_WAIT_TIME.set(&mut self.state.params, wait);
    }
}
