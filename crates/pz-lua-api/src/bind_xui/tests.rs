use super::*;
use pz_kahlua::{TableRef, Thread};
use pz_scripts::ScriptManager;

const SCRIPTS: &str = "module Base {
    xuiConfig project_zomboid {
        String { styleCell, }
        Double { width, itemheight, }
        Boolean { anchorLeft, }
        StringList { animationList, }
        FontType { font, }
        Color { backgroundColor, textColor, }
    }
    xuiGlobalColors test_colors { C_Back = 0:0:1:0.5, }
    xuiSkin base {
        ISButton { width = 20, font = UIFont.Large, }
        colors { C_Own = rgb:255:0:51, }
    }
    xuiSkin default {
        imports { base, }
        ISButton { anchorLeft = true, textColor = C_Own, backgroundColor = C_Back, }
        ISButton Big { width = 40, animationList = a:b : c, }
        ISPanel Plain { styleCell = nil, }
        entity { luaWindowClass = ISWin, components { Parts = PartsPanel:PartsStyle, } }
        entity Crate { displayName = Crate, }
    }
}";

fn run(t: &mut Thread, src: &str) -> Vec<Value> {
    let f = pz_kahlua::stdlib::load_string(src, Some("test"), t.globals().clone()).unwrap();
    t.call(&f, &[]).unwrap_or_else(|e| panic!("lua error: {e:?}"))
}

fn setup() -> Thread {
    let dir = std::env::temp_dir().join(format!("pz-bind-xui-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let file = dir.join("media/scripts/xui.txt");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, SCRIPTS).unwrap();
    let sm = ScriptManager::new();
    sm.load(&pz_fs::GameFs::new(dir.clone()), &[]).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
    crate::bind_registry::install_script_manager(Rc::new(sm));
    let mut t = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut t, None).unwrap();
    crate::bind_java_util::register(&mut t);
    crate::bind_misc_classes::register(&mut t, None);
    crate::bind_colors::register_with(&mut t, crate::bind_colors::Colors::default());
    crate::bind_globals::register(&mut t);
    let fonts: Vec<&'static str> = UIFont::ALL.iter().map(|f| f.name()).collect();
    expose_enum(&mut t, "zombie.ui.UIFont", &fonts);
    let components: Vec<&'static str> = ComponentType::ALL.iter().map(|c| c.name()).collect();
    expose_enum(&mut t, "zombie.entity.ComponentType", &components);
    register(&mut t);
    run(&mut t, "function assert(v, msg) if not v then error(msg or 'assertion failed!', 2) end return v end");
    t
}

#[test]
fn default_skin_styles_read_like_xui_skin_build() {
    let mut t = setup();
    let r = run(
        &mut t,
        r#"
        local n = 0
        local function a(v) n = n + 1 if not v then error('check ' .. n) end return v end
        a(Colors.GetColorByName("C_Back") ~= nil)
        local skin = XuiManager.GetDefaultSkin()
        a(not skin:isInvalidated())
        a(instanceof(skin, "XuiSkin"))
        a(skin:getName() == "default")
        local d = skin:getDefault("ISButton")
        local s = skin:get("ISButton", "Big")
        a(d and s and not skin:get("ISButton", "Nope") and not skin:getDefault("Nothing"))
        -- imported default first, overlaid by the importing skin
        a(d:getVar("width"):value() == 20 and d:getVar("width"):isValueSet())
        a(d:getVar("anchorLeft"):value() == true)
        a(d:getVar("font"):value() == UIFont.Large)
        -- a named style starts from the default style
        a(s:getVar("width"):value() == 40 and s:getVar("anchorLeft"):value() == true)
        local list = s:getVar("animationList"):value()
        a(list:size() == 3 and list:get(2) == "c")
        -- unset var: no entry; registered var with nil value stays set
        a(d:getVar("itemheight") == nil)
        local plain = skin:get("ISPanel", "Plain")
        a(plain:getVar("styleCell"):isValueSet() and plain:getVar("styleCell"):value() == nil)
        -- colours: skin colour map, imports, game colours
        local c = d:getVar("textColor")
        a(c:getType() == XuiVarType.Color and c:getR() == 1 and c:getG() == 0)
        a(math.abs(c:getB() - 0.2) < 1e-6)
        local b = d:getVar("backgroundColor")
        a(b:getB() == 1 and b:getA() == 0.5)
        a(skin:color("C_Own") ~= nil and skin:color("Missing") == nil)
        a(d:getVar("width"):getAutoApplyMode() == XuiAutoApply.IfSet)
        a(d:getVars():size() == 5)
        -- entity styles
        a(skin:getEntityUiStyle(nil):getLuaWindowClass() == "ISWin")
        a(skin:getEntityUiStyle("Crate"):getLuaWindowClass() == "ISWin")
        a(skin:getComponentUiStyle(nil, ComponentType.Parts):getLuaPanelClass() == "PartsPanel")
        a(skin:getComponentUiStyle(nil, ComponentType.Parts):isEnabled())
        a(XuiManager.GetSkin("nothing") == skin)
        return true
        "#,
    );
    assert!(matches!(r[0], Value::Bool(true)));
    run(&mut t, "oldSkin = XuiManager.GetDefaultSkin()");
    let previous = crate::bind_registry::shared_script_manager();
    crate::bind_registry::install_script_manager(previous.clone());
    run(&mut t, "assert(not oldSkin:isInvalidated())");
    crate::bind_registry::install_script_manager(Rc::new(ScriptManager::new()));
    run(&mut t, "assert(oldSkin:isInvalidated() and XuiManager.GetDefaultSkin() == nil)");
    crate::bind_registry::install_script_manager(previous);
    run(&mut t, "assert(XuiManager.GetDefaultSkin() ~= oldSkin and not XuiManager.GetDefaultSkin():isInvalidated())");
}
