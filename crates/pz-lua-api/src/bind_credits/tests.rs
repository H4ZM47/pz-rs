use super::*;
use pz_fs::GameFs;

fn temp_dir(tag: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("pz-bind-credits-{tag}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(dir.join("media")).unwrap();
    dir
}

fn lua(dir: &Path) -> LuaManager {
    let fs = Rc::new(RefCell::new(GameFs::new(dir)));
    LuaManager::init(fs).unwrap()
}

fn run(mgr: &mut LuaManager, src: &str) -> Vec<Value> {
    let f = pz_kahlua::stdlib::load_string(src, Some("test"), mgr.thread.globals().clone()).unwrap();
    mgr.thread.call(&f, &[]).unwrap()
}

/// A small hand-made set in the enums' shape.
fn sample() -> CreditsData {
    let n = |c: &str, name: &str, sort: Option<&str>| (c.to_string(), name.to_string(), sort.map(str::to_string));
    CreditsData {
        names: vec![
            n("SPIFFO", "Spiffo", None),
            n("ZED", "Zed \"Z\" Zulu", None),
            n("AITOR", "Aitor García de la Cruz", Some("García")),
            n("BRICK", "a_brick_fairy", None),
            n("GAMB", "Jones \"Gambarine\"", None),
        ],
        roles: vec![
            ("HEAD_RACCOON".into(), true, vec![0]),
            ("FOUNDERS".into(), false, vec![1, 3, 2]),
            ("ENGINEERS".into(), true, vec![1, 3, 2, 4]),
            ("LOCALIZATION".into(), true, vec![]),
        ],
        groups: vec![
            ("EXECUTIVES_AND_LEADERSHIP".into(), None, vec![0, 1]),
            ("VERTEX_BREAK".into(), Some("media/ui/Logos/VertexBreak_credits.png".into()), vec![2]),
            ("LOCALIZATION".into(), None, vec![3]),
            ("TOOLS".into(), None, vec![]),
        ],
    }
}

fn display(d: &CreditsData, names: Vec<usize>) -> Vec<&str> {
    names.into_iter().map(|i| d.names[i].1.as_str()).collect()
}

#[test]
fn sorting_and_titles() {
    let d = sample();
    assert_eq!(d.name_sort(2), "GARCÍA");
    assert_eq!(d.name_sort(0), "SPIFFO");
    assert_eq!(d.name_sort(4), "\"GAMBARINE\"");
    assert_eq!(display(&d, d.role_names(1)), ["Zed \"Z\" Zulu", "a_brick_fairy", "Aitor García de la Cruz"]);
    // Upper-cased sort keys compared as Java strings: "\"GAMBARINE\"" <
    // "A_BRICK_FAIRY" < "GARCÍA" < "ZULU".
    assert_eq!(
        display(&d, d.role_names(2)),
        ["Jones \"Gambarine\"", "a_brick_fairy", "Aitor García de la Cruz", "Zed \"Z\" Zulu"]
    );
    assert!(d.role_names(3).is_empty());
    assert_eq!(d.role_title(0), "credits_role.head_raccoon");
    assert_eq!(d.group_title(0), "credits_role_group.executives_and_leadership");
}

#[test]
fn lua_api_matches_credits_screen_use() {
    let dir = temp_dir("lua");
    let mut mgr = lua(&dir);
    register_data(&mut mgr, Rc::new(sample()));
    let r = run(
        &mut mgr,
        r#"
        local all = CreditsRoleGroup.getAll()
        local g = all:get(0)
        local roles = g:getRoles()
        local loc = CreditsRoleGroup.LOCALIZATION:getRoles():get(0)
        local founders = roles:get(1)
        return all:size(), g:getTitle(), g:getLogo() == nil, CreditsRoleGroup.VERTEX_BREAK:getLogo(),
            roles:size(), loc == CreditsRole.LOCALIZATION, founders:getNames():get(0):getName(),
            founders:getNames() == founders:getNames(), CreditsName.SPIFFO:getSort(),
            tostring(CreditsRoleGroup.TOOLS), CreditsRoleGroup.TOOLS:getRoles():size(), CreditsRole.TRANSLATOR,
            CreditsName.valueOf("SPIFFO"):getName()
    "#,
    );
    assert_eq!(r[0].as_num(), Some(4.0));
    assert_eq!(r[1].as_str(), Some("credits_role_group.executives_and_leadership"));
    assert!(r[2].truthy());
    assert_eq!(r[3].as_str(), Some("media/ui/Logos/VertexBreak_credits.png"));
    assert_eq!(r[4].as_num(), Some(2.0));
    assert!(r[5].truthy());
    assert_eq!(r[6].as_str(), Some("Zed \"Z\" Zulu"));
    assert!(r[7].truthy());
    assert_eq!(r[8].as_str(), Some("SPIFFO"));
    assert_eq!(r[9].as_str(), Some("TOOLS"));
    assert_eq!(r[10].as_num(), Some(0.0));
    assert_eq!(r[11].as_str(), Some("Translator"));
    assert_eq!(r[12].as_str(), Some("Spiffo"));
    // With no Translate folder the translator credits are empty.
    let r = run(&mut mgr, "return CreditsRole.getTranslatorCredits()");
    assert!(r[0].as_table().unwrap().is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}

fn lang(name: &str, text: &str) -> Rc<Language> {
    Rc::new(Language { name: name.into(), text: text.into(), base: None, azerty: false })
}

#[test]
fn translator_credit_files() {
    let dir = temp_dir("tr");
    let t = dir.join(TRANSLATION_FOLDER);
    for (d, json) in [
        ("DE", Some("{\"Translator\": \"Anna\\nBen\\n\\n\"}")),
        ("EN", Some("{\"Translator\": \"Nobody\"}")),
        ("FR", Some("{ Translator: '' }")),
        ("XX", Some("{\"Translator\": \"Unknown language\"}")),
        ("IT", None),
    ] {
        std::fs::create_dir_all(t.join(d)).unwrap();
        if let Some(j) = json {
            std::fs::write(t.join(d).join(CREDITS_TRANSLATOR_JSON), j).unwrap();
        }
    }
    let langs = [lang("EN", "English"), lang("DE", "Deutsch"), lang("FR", "Francais"), lang("IT", "Italiano")];
    let lookup = |n: &str| langs.iter().find(|l| l.name.eq_ignore_ascii_case(n)).cloned();
    assert_eq!(translator_credits_list(&dir, "DE").unwrap(), ["Anna", "Ben"]);
    assert!(translator_credits_list(&dir, "FR").unwrap().is_empty());
    assert!(translator_credits_list(&dir, "IT").unwrap().is_empty());
    let all = translator_credits(&dir, lookup, "EN");
    assert_eq!(all, vec![("Deutsch".to_string(), vec!["Anna".to_string(), "Ben".to_string()])]);
    // A file without the key throws from the list method and stops the
    // walk in getTranslatorCredits.
    std::fs::write(t.join("IT").join(CREDITS_TRANSLATOR_JSON), "{\"Other\": 1}").unwrap();
    let e = translator_credits_list(&dir, "IT").unwrap_err();
    assert_eq!(e.class, "org.json.JSONException");
    let all = translator_credits(&dir, lookup, "EN");
    assert!(all.len() <= 1);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The shipped translator files all load.
#[test]
fn game_translator_credits() {
    let Some(game) = std::env::var_os("PZ_GAME_DIR") else { return };
    let game = PathBuf::from(game);
    let mut languages = pz_text::translator::Languages::default();
    languages.init(&game, &[]);
    let all = translator_credits(&game, |n| languages.by_name(n), "EN");
    assert!(all.len() > 10, "{all:?}");
    assert!(all.iter().all(|(_, l)| !l.is_empty() && l.iter().all(|s| !s.contains('\n'))));
}

/// The game's enums read from its jar: counts, then (with
/// `PZ_CREDITS_DUMP=<file>`) every group, role and name in the format of
/// the reflection dump of the original classes, for diffing.
#[test]
fn game_jar_credits() {
    let Some(game) = std::env::var_os("PZ_GAME_DIR") else { return };
    let jar = Jar::open(PathBuf::from(game).join("projectzomboid.jar")).unwrap();
    let d = CreditsData::load(&jar).unwrap();
    assert_eq!((d.names.len(), d.roles.len(), d.groups.len()), (270, 80, 18));
    let founders = d.roles.iter().position(|r| r.0 == "FOUNDERS").unwrap();
    assert!(!d.roles[founders].1);
    let Some(out) = std::env::var_os("PZ_CREDITS_DUMP") else { return };
    let mut s = String::new();
    for (g, (gname, logo, roles)) in d.groups.iter().enumerate() {
        s += &format!("G {gname}|{}|{}\n", d.group_title(g), logo.as_deref().unwrap_or("null"));
        for &r in roles {
            s += &format!(" R {}|{}\n", d.roles[r].0, d.role_title(r));
            for n in d.role_names(r) {
                s += &format!("  N {}|{}|{}\n", d.names[n].0, d.names[n].1, d.name_sort(n));
            }
        }
    }
    std::fs::write(out, s).unwrap();
}
