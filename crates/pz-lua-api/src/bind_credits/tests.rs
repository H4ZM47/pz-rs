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

fn idx(name: &str) -> usize {
    data::NAMES.iter().position(|n| n.0 == name).unwrap()
}

fn role(name: &str) -> usize {
    data::ROLES.iter().position(|n| n.0 == name).unwrap()
}

fn display(names: Vec<usize>) -> Vec<&'static str> {
    names.into_iter().map(|i| data::NAMES[i].1).collect()
}

#[test]
fn constants_sorting_and_titles() {
    assert_eq!((data::NAMES.len(), data::ROLES.len(), data::GROUPS.len()), (270, 80, 18));
    assert_eq!(name_sort(idx("AITOR_CRUZ")), "GARCÍA");
    assert_eq!(name_sort(idx("THOMAS_DEYN")), "DE");
    assert_eq!(name_sort(idx("SPIFFO")), "SPIFFO");
    assert_eq!(name_sort(idx("JONES_GAMBARINE")), "\"GAMBARINE\"");
    assert_eq!(name_sort(idx("SYED_BILAL_AHMED")), "(MAIYIN)");
    // FOUNDERS and WRITER keep their order; the rest sort by surname.
    assert_eq!(
        display(role_names(role("FOUNDERS"))),
        ["Chris \"Lemmy\" Simpson", "Andy \"Captain Binky\" Hodgetts", "Will Porter", "Marina \"Mash\" Siu-Chong"]
    );
    assert_eq!(display(role_names(role("TMG_PT"))), ["Jorge Guedes", "Daniel Marques"]);
    assert_eq!(
        display(role_names(role("COMMUNITY_MODERATORS"))),
        ["a_brick_fairy", "Mason Parada", "E \"Frogski\" Read", "James \"Mr Sunshine\" Stretton"]
    );
    assert!(role_names(role("LOCALIZATION")).is_empty());
    assert_eq!(role_title(role("HEAD_RACCOON")), "credits_role.head_raccoon");
    assert_eq!(group_title(0), "credits_role_group.executives_and_leadership");
}

#[test]
fn lua_api_matches_credits_screen_use() {
    let dir = temp_dir("lua");
    let mut mgr = lua(&dir);
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
    assert_eq!(r[0].as_num(), Some(18.0));
    assert_eq!(r[1].as_str(), Some("credits_role_group.executives_and_leadership"));
    assert!(r[2].truthy());
    assert_eq!(r[3].as_str(), Some("media/ui/Logos/VertexBreak_credits.png"));
    assert_eq!(r[4].as_num(), Some(6.0));
    assert!(r[5].truthy());
    assert_eq!(r[6].as_str(), Some("Chris \"Lemmy\" Simpson"));
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

/// `PZ_CREDITS_DUMP=<file>`: the groups, roles and names in the format of
/// the reflection dump of the original classes, for diffing.
#[test]
fn dump_for_oracle() {
    let Some(out) = std::env::var_os("PZ_CREDITS_DUMP") else { return };
    let mut s = String::new();
    for (g, (gname, logo, roles)) in data::GROUPS.iter().enumerate() {
        s += &format!("G {gname}|{}|{}\n", group_title(g), logo.unwrap_or("null"));
        for &r in roles.iter() {
            s += &format!(" R {}|{}\n", data::ROLES[r].0, role_title(r));
            for n in role_names(r) {
                s += &format!("  N {}|{}|{}\n", data::NAMES[n].0, data::NAMES[n].1, name_sort(n));
            }
        }
    }
    std::fs::write(out, s).unwrap();
}
