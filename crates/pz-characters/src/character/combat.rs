//! Combat: `IsoGameCharacter`/`IsoLivingCharacter`/`IsoZombie`/`IsoPlayer`
//! hit helpers (`Hit`, `processHitDamage`, `hitConsequences`, `Kill`) and
//! `zombie.CombatManager`'s melee resolution.

use super::state::StateCx;
use super::{GameCharacter, SimEvent};

impl<I: 'static> GameCharacter<I> {
    /// `Kill(killer)`: health to zero and, once, `onKilled`.
    pub fn kill(&mut self, cx: &mut StateCx<'_, I>) {
        let killer = self.vars.attacked_by;
        self.kill_by(killer, cx);
    }

    /// `Kill(killer, weapon, isGory, null)`.
    pub fn kill_by(&mut self, killer: Option<i32>, cx: &mut StateCx<'_, I>) {
        self.vars.attacked_by = killer;
        self.set_health(cx.ctx, 0.0);
        self.body_damage.overall_body_health = 0.0;
        if !self.vars.on_kill_done {
            cx.ctx.event(SimEvent::Killed { attacker: killer });
            self.vars.on_kill_done = true;
        }
    }
}
