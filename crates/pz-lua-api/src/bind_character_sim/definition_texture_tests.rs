use super::tests::{run, setup_with};
use super::*;

struct Restore(Option<pz_render::TextureRefLoader>);
impl Drop for Restore {
    fn drop(&mut self) {
        drop(pz_render::install_texture_ref_loader(self.0.take()));
        pz_inventory::component_host::drain_retired_components();
    }
}

#[test]
fn definition_textures_use_the_constructor_lookups() {
    let lookups: Rc<RefCell<Vec<String>>> = Rc::default();
    let seen = lookups.clone();
    let _restore = Restore(pz_render::install_texture_ref_loader(Some(Rc::new(
        move |lookup, name: &str| {
            seen.borrow_mut().push(format!("{lookup:?} {name}"));
            let known = ["media/ui/Traits/trait_generic.png", "media/ui/Traits/trait_strong.png", "media/ui/prof.png"];
            Ok(known.contains(&name).then(|| {
                Rc::new(pz_render::LuaTexture::new(pz_render::Texture::empty(name)))
            }))
        },
    ))));
    let (mut thread, _player, _) = setup_with(ScriptManager::new());
    run(&mut thread, r#"
        local strong = CharacterTraitDefinition.new(CharacterTrait.STRONG, "UI_strong", 4, "", false, false)
        assert(tostring(strong:getTexture()):find("media/ui/Traits/trait_strong.png", 1, true) ~= nil)
        local quick = CharacterTraitDefinition.new(CharacterTrait.FAST_LEARNER, "UI_quick", 4, "", false, false)
        assert(tostring(quick:getTexture()):find("media/ui/Traits/trait_generic.png", 1, true) ~= nil, "generic fallback")
        local prof = CharacterProfessionDefinition.new(nil, "UI_prof", 0, "", "media/ui/prof.png")
        assert(tostring(prof:getTexture()):find("media/ui/prof.png", 1, true) ~= nil)
        assert(CharacterProfessionDefinition.new(nil, "UI_none", 0, "", ""):getTexture() == nil)
        assert(CharacterProfessionDefinition.new(nil, "UI_missing", 0, "", "media/ui/missing.png"):getTexture() == nil)
    "#).unwrap();
    let lookups = lookups.borrow();
    assert!(lookups.iter().any(|l| l.starts_with("Shared") && l.ends_with("trait_strong.png")), "{lookups:?}");
    assert!(lookups.iter().any(|l| l.starts_with("Try") && l.ends_with("prof.png")), "{lookups:?}");
}
