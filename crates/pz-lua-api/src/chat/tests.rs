use super::base::ChatTab;
use super::element::ChatElement;
use super::utility::{
    parse_string_for_chat_bubble, parse_string_for_chat_log, scramble_string, scramble_value, Speaker,
};
use super::*;
use crate::hud::host::{install, HudGameTime, HudHost};
use pz_core::config::float_to_string;
use pz_core::debug::FixedClock;
use pz_kahlua::TableRef;
use pz_render::Rgba;
use pz_text::UIFont;
use pz_ui::{with_ui, TextAlign, UiDrawCmd};
use std::cell::Cell;
use std::rc::Weak;

/// A local player: speaker (`ChatElementOwner`) and listener.
struct TestPlayer {
    num: usize,
    name: String,
    x: Cell<f32>,
    y: Cell<f32>,
    z: Cell<f32>,
    square: Cell<Option<ChatSquare>>,
    deaf: Cell<bool>,
    radio: Cell<Option<RadioDeviceState>>,
    element: Rc<RefCell<ChatElement>>,
}

impl ChatElementOwner for TestPlayer {
    fn get_x(&self) -> f32 {
        self.x.get()
    }
    fn get_y(&self) -> f32 {
        self.y.get()
    }
    fn get_z(&self) -> f32 {
        self.z.get()
    }
    fn get_square(&self) -> Option<ChatSquare> {
        self.square.get()
    }
    fn is_player(&self, n: usize) -> bool {
        n == self.num
    }
}

impl ChatPlayer for TestPlayer {
    fn player_num(&self) -> usize {
        self.num
    }
    fn username(&self) -> String {
        self.name.clone()
    }
    fn chat_element(&self) -> Rc<RefCell<ChatElement>> {
        self.element.clone()
    }
    fn speak_colour(&self) -> (f32, f32, f32) {
        (0.5, 0.25, 1.0)
    }
    fn get_x(&self) -> f32 {
        self.x.get()
    }
    fn get_y(&self) -> f32 {
        self.y.get()
    }
    fn get_z(&self) -> f32 {
        self.z.get()
    }
    fn get_square(&self) -> Option<ChatSquare> {
        self.square.get()
    }
    fn is_deaf(&self) -> bool {
        self.deaf.get()
    }
    fn equipped_radio(&self) -> Option<RadioDeviceState> {
        self.radio.get()
    }
}

fn player(num: usize, name: &str, x: f32, y: f32) -> Rc<TestPlayer> {
    Rc::new_cyclic(|w: &Weak<TestPlayer>| {
        let owner: Weak<dyn ChatElementOwner> = w.clone();
        TestPlayer {
            num,
            name: name.into(),
            x: Cell::new(x),
            y: Cell::new(y),
            z: Cell::new(0.0),
            square: Cell::new(None),
            deaf: Cell::new(false),
            radio: Cell::new(None),
            element: Rc::new(RefCell::new(ChatElement::new(owner, 1, Some("character")))),
        }
    })
}

struct Players(Vec<Rc<TestPlayer>>);

impl ChatPlayers for Players {
    fn num_players(&self) -> usize {
        self.0.len()
    }
    fn player(&self, i: usize) -> Option<Rc<dyn ChatPlayer>> {
        self.0.get(i).map(|p| p.clone() as Rc<dyn ChatPlayer>)
    }
    fn instance(&self) -> Option<Rc<dyn ChatPlayer>> {
        self.player(0)
    }
}

struct Multiplier(f32);
impl HudGameTime for Multiplier {
    fn get_multiplier(&self) -> f32 {
        self.0
    }
}

/// Fresh thread state: the given players, multiplier 1, 13:05 local.
fn setup(ps: &[Rc<TestPlayer>]) {
    install(HudHost { game_time: Box::new(Multiplier(1.0)), ..HudHost::default() });
    install_players(Rc::new(Players(ps.to_vec())));
    set_clock(Rc::new(FixedClock::new((13 * 60 + 5) * 60_000, 0)));
    crate::bind_core::shared_core().borrow_mut().debug = false;
}

fn original(p: &TestPlayer, player: usize, i: usize) -> String {
    p.element.borrow().line(player, i).map(|l| l.get_original().to_string()).unwrap_or_default()
}

fn clock(p: &TestPlayer, player: usize, i: usize) -> f32 {
    p.element.borrow().line(player, i).map_or(-1.0, |l| l.get_internal_clock())
}

fn add(p: &TestPlayer, text: &str, tag: &str) {
    p.element.borrow_mut().add_chat_line_full(
        text,
        1.0,
        1.0,
        1.0,
        UIFont::Dialogue,
        30.0,
        tag,
        false,
        false,
        false,
        false,
        false,
        true,
    );
}

#[test]
fn element_lines_rotate_and_expire() {
    let bob = player(0, "Bob", 10.0, 10.0);
    setup(std::slice::from_ref(&bob));
    let el = bob.element.clone();
    assert_eq!(el.borrow().get_max_chat_lines(), 1);
    assert_eq!(el.borrow().get_talker_type(), "character");
    el.borrow_mut().set_max_chat_lines(5);

    add(&bob, "hello", "default");
    assert_eq!(original(&bob, 0, 0), "hello");
    assert_eq!(clock(&bob, 0, 0), 314.0);
    assert!(el.borrow().get_has_chat_to_display());
    assert_eq!(el.borrow().get_say_line(), Some("hello"));

    el.borrow_mut().update();
    assert_eq!(clock(&bob, 0, 0), 312.75);
    assert!(el.borrow().is_speaking() && el.borrow().is_speaking_npc());
    assert_eq!(el.borrow().get_say_line_tag(), "default");

    add(&bob, "second", "default");
    assert_eq!((original(&bob, 0, 0), original(&bob, 0, 1)), ("second".into(), "hello".into()));
    el.borrow_mut().update();
    // Each older line runs 1.2x faster.
    assert_eq!(clock(&bob, 0, 0), 312.75);
    assert_eq!(clock(&bob, 0, 1), 311.25);

    // Speaking ends halfway through the newest line's time.
    let mut ticks = 1;
    while el.borrow().is_speaking() {
        el.borrow_mut().update();
        ticks += 1;
    }
    assert_eq!(ticks, 126);
    assert!(el.borrow().is_speaking_npc());
    assert_eq!(el.borrow().get_say_line(), None);
    assert_eq!(el.borrow().get_say_line_tag(), "");
    while el.borrow().get_has_chat_to_display() {
        el.borrow_mut().update();
    }
    assert_eq!(clock(&bob, 0, 0), 0.0);
    assert_eq!(el.borrow().history_val(), 0.0);

    // The newest radio line does not make the speaker speak.
    add(&bob, "static", "radio");
    el.borrow_mut().update();
    assert!(!el.borrow().is_speaking());
    assert!(el.borrow().get_has_chat_to_display());

    // Capped at 10; below 1 is kept as in the original.
    el.borrow_mut().set_max_chat_lines(20);
    assert_eq!(el.borrow().get_max_chat_lines(), 10);
    el.borrow_mut().set_max_chat_lines(0);
    assert_eq!(el.borrow().get_max_chat_lines(), 0);
    add(&bob, "nowhere", "default");
    assert!(el.borrow().line(0, 0).is_none());
}

#[test]
fn element_say_debug_and_clear() {
    let bob = player(0, "Bob", 0.0, 0.0);
    setup(std::slice::from_ref(&bob));
    let el = bob.element.clone();
    el.borrow_mut().say_debug(0, "debug");
    assert_eq!(original(&bob, 0, 0), "debug");
    assert_eq!(el.borrow().line(0, 0).unwrap().get_default_font_enum(), UIFont::Medium);
    el.borrow_mut().update();
    el.borrow_mut().say_debug(0, "debug");
    assert_eq!(clock(&bob, 0, 0), 314.0);
    el.borrow_mut().say_debug(1, "out of range");
    assert_eq!(el.borrow().get_say_line(), Some("debug"));
    el.borrow_mut().clear(0);
    assert_eq!(clock(&bob, 0, 0), 0.0);
    assert_eq!(original(&bob, 0, 0), "");
    assert!(!el.borrow().get_has_chat_to_display());
}

#[test]
fn element_hearing_range_and_scramble() {
    let bob = player(0, "Bob", 0.0, 0.0);
    let near = player(1, "Near", 10.0, 0.0);
    let far = player(2, "Far", 40.0, 0.0);
    let mid = player(3, "Mid", 0.0, 25.0);
    setup(&[bob.clone(), near.clone(), far.clone(), mid.clone()]);
    pz_core::random::set_seed(3);
    add(&bob, "one two three four five six", "default");
    assert_eq!(original(&bob, 0, 0), "one two three four five six");
    assert_eq!(original(&bob, 1, 0), "one two three four five six");
    assert_eq!(original(&bob, 2, 0), "");
    // 25 of 30 tiles: (25 - 18) / 12 scrambled.
    assert!(original(&bob, 3, 0).contains("..."));

    let speaker = |sq| Speaker { x: 0.0, y: 0.0, z: 0.0, square: sq };
    let room = Some(ChatSquare { building: Some(1), room: Some(7) });
    mid.square.set(room);
    assert_eq!(scramble_value(speaker(room), &*mid, 30.0, true), 0.0);
    mid.square.set(Some(ChatSquare { building: None, room: None }));
    // Inside versus outside halves the range: 25 >= 15.
    assert_eq!(scramble_value(speaker(room), &*mid, 30.0, true), 1.0);
    near.z.set(1.0);
    near.square.set(Some(ChatSquare::default()));
    assert_eq!(scramble_value(speaker(Some(ChatSquare::default())), &*near, 30.0, true), 0.2);
    assert_eq!(scramble_value(speaker(None), &*far, 30.0, false), 1.0);

    // Deaf listeners miss radios, not people.
    near.deaf.set(true);
    near.z.set(0.0);
    near.square.set(None);
    add(&bob, "spoken", "default");
    assert_eq!(original(&bob, 1, 0), "spoken");

    assert_eq!(scramble_string("a b", 0, "..."), "a b");
    assert_eq!(scramble_string("a b", 100, "..."), "...");
    pz_core::random::set_seed(1);
    let s = scramble_string("[col=1,2,3]alpha beta gamma delta", 50, "...");
    // The leading tag ends a (scrambled or kept) empty word first.
    assert!(s.contains("[col=1,2,3]"), "{s}");
}

/// 8 px a character, 16 px lines.
struct Mono;
impl pz_ui::TextRenderer for Mono {
    fn measure_x(&self, _: Option<UIFont>, s: &str) -> i32 {
        8 * s.encode_utf16().count() as i32
    }
    fn measure_y(&self, f: Option<UIFont>, _: &str) -> i32 {
        self.font_height(f)
    }
    fn font_height(&self, _: Option<UIFont>) -> i32 {
        16
    }
    #[allow(clippy::too_many_arguments)]
    fn draw(
        &self,
        out: &mut pz_ui::SpriteRenderer,
        font: Option<UIFont>,
        x: f64,
        y: f64,
        zoom: f32,
        text: &str,
        color: Rgba,
        align: TextAlign,
        trimmed: bool,
    ) {
        out.push(UiDrawCmd::Text { font, x: x as f32, y: y as f32, zoom, text: Rc::from(text), color, align, trimmed });
    }
}

#[test]
fn element_render_batch_stacks_lines_upwards() {
    let bob = player(0, "Bob", 0.0, 0.0);
    setup(std::slice::from_ref(&bob));
    pz_ui::UIManager::set_text_renderer(Some(Rc::new(Mono)));
    crate::bind_core::shared_core().borrow_mut().set_screen_size(1280, 720);
    with_ui(|m| m.sprite.clear());
    bob.element.borrow_mut().set_max_chat_lines(5);
    add(&bob, "older", "default");
    add(&bob, "newer", "default");
    ChatElement::render_batched(&bob.element, 0, 400, 300, false);
    ChatElement::render_batch(0);
    let texts: Vec<(String, f32, f32, f32)> = with_ui(|m| {
        m.sprite
            .cmds
            .iter()
            .filter_map(|c| match c {
                UiDrawCmd::Text { text, x, y, color, .. } => Some((text.to_string(), *x, *y, color.3)),
                _ => None,
            })
            .collect()
    });
    // Dialogue lines have no outline; centred on x, 17 px apart.
    assert_eq!(texts, vec![("newer".into(), 380.0, 283.0, 1.0), ("older".into(), 380.0, 266.0, 1.0)]);
    // Rendering drained the batch.
    with_ui(|m| m.sprite.clear());
    ChatElement::render_batch(0);
    assert!(with_ui(|m| m.sprite.cmds.is_empty()));
    ChatElement::render_batched(&bob.element, 0, 400, 300, false);
    ChatElement::no_render(0);
    ChatElement::render_batch(0);
    assert!(with_ui(|m| m.sprite.cmds.is_empty()));
}

fn init_bob() -> (Rc<TestPlayer>, Rc<TestPlayer>) {
    let bob = player(0, "Bob", 0.0, 0.0);
    let ann = player(1, "Ann", 5.0, 0.0);
    setup(&[bob.clone(), ann.clone()]);
    bob.element.borrow_mut().set_max_chat_lines(5);
    with_chat_manager(|m| m.init(true, bob.clone())).unwrap();
    (bob, ann)
}

#[test]
fn message_prefixes() {
    let (_bob, _ann) = init_bob();
    let say = with_chat_manager(|m| m.chat(ChatType::Say)).unwrap();
    let m = say.create_message(Some("hi <b>")).unwrap();
    assert_eq!(m.get_author().as_deref(), Some("Bob"));
    assert_eq!(m.get_datetime_str(), "1:5");
    assert_eq!(
        m.get_text_with_prefix(),
        "<RGB:1.0,1.0,1.0> <SIZE:medium> [13:05][UI_chat_local_chat_title_id][Bob]:  hi &lt;b&gt;"
    );
    say.set_show_timestamp(false);
    say.set_show_title(false);
    say.set_font_size("Large");
    assert_eq!(say.get_message_prefix(&m), "<RGB:1.0,1.0,1.0> <SIZE:large> [Bob]: ");
    say.set_font_size("huge");
    assert_eq!(say.get_font_size(), "notdefine");
    // The say chat's log turns *colour* markup into tags.
    m.set_text(Some("*255,0,0*red *x*"));
    assert!(m.get_text_with_prefix().ends_with("]:   <RGB:1.0,0.0,0.0>red *x*"), "{}", m.get_text_with_prefix());

    let shout = with_chat_manager(|m| m.chat(ChatType::Shout)).unwrap();
    let m = shout.create_message(Some("HEY")).unwrap();
    assert_eq!(
        m.get_text_with_prefix(),
        "<RGB:1.0,0.2,0.2> <SIZE:medium> [13:05][UI_chat_local_chat_title_id][Bob]:  HEY"
    );

    let general = with_chat_manager(|m| m.chat(ChatType::General)).unwrap();
    let m = general.create_message(Some("all")).unwrap();
    assert_eq!(
        m.get_text_with_prefix(),
        "<RGB:1.0,1.0,1.0> <SIZE:medium> [13:05][UI_chat_general_chat_title_id][Bob]:  all"
    );
    m.make_from_discord();
    let f = |v: i32| float_to_string(v as f32 / 255.0);
    assert!(m.get_text_with_prefix().starts_with(&format!("<RGB:{},{},{}> <SIZE:medium> [", f(114), f(137), f(218))));

    let radio = with_chat_manager(|m| m.chat(ChatType::Radio)).unwrap();
    let m = with_chat_manager(|m| m.create_radiostation_message("news", 98200)).unwrap();
    assert_eq!((m.get_author().as_deref(), m.get_radio_channel()), (Some(""), 98200));
    assert!(!m.is_should_attract_zombies() && !m.is_show_in_chat());
    assert_eq!(
        radio.get_message_prefix(&m),
        "<RGB:0.7,0.7,0.7> <SIZE:medium> [13:05][UI_chat_radio_chat_title_id] Radio  (98.2 MHz): "
    );
    m.set_radio_channel(-1);
    assert!(radio.get_message_prefix(&m).ends_with(" Radio  (0.-1 MHz): "));

    let s = say.create_server_message(Some("srv"));
    assert_eq!(s.get_author().as_deref(), Some("Server"));
    assert!(s.is_server_author() && s.is_server_message());
    assert_eq!(s.set_author(Some("x")).unwrap_err().class, "java.lang.UnsupportedOperationException");
    let c = s.clone_message();
    assert!(c.is_server_message() && c.get_text().as_deref() == Some("srv"));

    assert_eq!(parse_string_for_chat_bubble("*255,0,0*hi*0,255,0*yo"), "[col=255,0,0]hi[/][col=0,255,0]yo");
    assert_eq!(parse_string_for_chat_bubble("a*b[br/]"), "a*b");
    assert_eq!(parse_string_for_chat_bubble("*1,2,3*x*open"), "[col=1,2,3]x*open[/]");
    assert_eq!(parse_string_for_chat_bubble("*music*"), "[img=music]");
    assert_eq!(parse_string_for_chat_log("*300,0,0*no"), "*300,0,0*no");
}

#[test]
fn manager_single_player_init_and_say_shout() {
    let (bob, ann) = init_bob();
    with_chat_manager(|m| {
        assert!(m.is_single_player_mode() && !m.is_working());
        for t in [ChatType::Say, ChatType::General, ChatType::Shout, ChatType::Radio, ChatType::Admin] {
            assert!(m.chat(t).is_some(), "{t:?}");
        }
        assert!(m.chat(ChatType::Server).is_none() && m.chat(ChatType::Faction).is_none());
        assert_eq!(m.get_tab_name(3), "3");
        assert_eq!(m.focus_on_tab(0).unwrap_err().class, "java.lang.RuntimeException");
        assert!(m.get_focus_tab().is_none());
    });
    let say = with_chat_manager(|m| m.chat(ChatType::Say)).unwrap();
    assert!(say.get_tab_id().is_err());
    assert_eq!((say.get_range(), say.get_zombie_attraction_range()), (30.0, 15.0));
    assert!(say.is_sending_to_radio());

    with_chat_manager(|m| m.send_message_to_chat(ChatType::Say, "  hello  ")).unwrap();
    assert_eq!(original(&bob, 0, 0), "hello");
    assert_eq!(original(&bob, 1, 0), "hello");
    assert_eq!(say.message_count(), 1);
    with_chat_manager(|m| m.send_message_to_chat(ChatType::Say, "   ")).unwrap();
    assert_eq!(say.message_count(), 1);

    with_chat_manager(|m| m.send_message_to_chat_as("Bob", ChatType::Shout, "HEY")).unwrap();
    assert_eq!(original(&bob, 0, 0), "HEY");
    assert_eq!(original(&bob, 0, 1), "hello");

    // Another local player's message speaks through their own element.
    with_chat_manager(|m| m.send_message_to_chat_as("Ann", ChatType::Say, "from ann")).unwrap();
    assert_eq!(original(&ann, 0, 0), "from ann");
    assert_eq!(original(&bob, 0, 0), "HEY");

    with_chat_manager(|m| m.show_info_message_from("Bob", "info")).unwrap();
    assert_eq!(original(&bob, 0, 0), "info");

    // Radio messages are only for the chat window, which SP has none of.
    let radio = with_chat_manager(|m| m.chat(ChatType::Radio)).unwrap();
    with_chat_manager(|m| m.show_radio_message_text("radio talk", 98200)).unwrap();
    with_chat_manager(|m| m.show_static_radio_sound("<bzzt>")).unwrap();
    assert_eq!(radio.message_count(), 0);
    assert_eq!(original(&bob, 0, 0), "info");
    // Sending to radio directly is refused (an exception in debug).
    with_chat_manager(|m| m.send_message_to_chat(ChatType::Radio, "x")).unwrap();
    crate::bind_core::shared_core().borrow_mut().debug = true;
    let e = with_chat_manager(|m| m.send_message_to_chat(ChatType::Radio, "x")).unwrap_err();
    assert_eq!(e.class, "java.lang.IllegalArgumentException");
    let e = with_chat_manager(|m| m.send_message_to_chat(ChatType::Faction, "x")).unwrap_err();
    assert_eq!(e.message, "Chat 'faction' is null. Chat should be init before use!");
    crate::bind_core::shared_core().borrow_mut().debug = false;
    // No server chat to report the disabled chat in.
    let e = with_chat_manager(|m| m.send_message_to_chat(ChatType::Faction, "x")).unwrap_err();
    assert_eq!(e.class, "java.lang.NullPointerException");
    assert!(with_chat_manager(|m| m.show_server_chat_message("hi")).is_err());
    let e = with_chat_manager(|m| m.send_message_to_chat(ChatType::Whisper, "x")).unwrap_err();
    assert_eq!(e.message, "Whisper not unique chat");

    with_chat_manager(|m| {
        assert!(m.is_player_can_use_chat(ChatType::Say).unwrap());
        assert!(m.is_player_can_use_chat(ChatType::General).unwrap());
        assert!(!m.is_player_can_use_chat(ChatType::Radio).unwrap());
        assert!(!m.is_player_can_use_chat(ChatType::Admin).unwrap());
        assert!(!m.is_player_can_use_chat(ChatType::NotDefined).unwrap());
    });
    bob.radio.set(Some(RadioDeviceState { turned_on: true, two_way: true, portable: true, mic_muted: false }));
    assert!(with_chat_manager(|m| m.is_player_can_use_chat(ChatType::Radio)).unwrap());

    with_chat_manager(|m| m.update_chat_settings(Some("large"), true, false));
    assert_eq!(crate::bind_core::shared_core().borrow().get_option_chat_font_size(), "large");
    // The single-player chats keep their own settings.
    assert_eq!(say.get_font_size(), "medium");

    // Re-init replaces the chats.
    with_chat_manager(|m| m.init(true, bob.clone())).unwrap();
    let say2 = with_chat_manager(|m| m.chat(ChatType::Say)).unwrap();
    assert!(!Rc::ptr_eq(&say, &say2));
    assert_eq!(say.message_count(), 0);
}

fn run(t: &mut Thread, src: &str) {
    let p = pz_kahlua::compiler::compile(src, Some("test"), Some("test"), None).unwrap();
    let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(Rc::new(p), t.globals().clone()));
    if let Err((e, tr)) = t.pcall(&f, &[]) {
        panic!("lua error: {e:?}\n{tr}");
    }
}

#[test]
fn lua_globals_and_methods() {
    let (bob, _ann) = init_bob();
    let mut t = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut t, None).unwrap();
    lua::register(&mut t);
    let events: Rc<RefCell<Vec<String>>> = Rc::default();
    let ev = events.clone();
    install_event_sink(Rc::new(move |_t: &mut Thread, name: &str, args: &[Value]| {
        ev.borrow_mut().push(format!("{name} {}", args.len()));
    }));
    let say = with_chat_manager(|m| m.chat(ChatType::Say)).unwrap();
    t.globals().set_str("testChat", Value::Object(say.clone()));
    run(
        &mut t,
        r#"
        processSayMessage("  hi there ")
        processSayMessage("")
        processShoutMessage(nil)
        local m = testChat:createMessage("yo <x>")
        r_text, r_author, r_inchat = m:getText(), m:getAuthor(), m:isShowInChat()
        r_prefix = m:getTextWithPrefix()
        r_repl = m:getTextWithReplacedParentheses()
        r_title, r_type, r_mode = testChat:getTitleID(), tostring(testChat:getType()), tostring(testChat:getMode())
        r_range, r_radio = testChat:getRange(), testChat:isSendingToRadio()
        r_tab = pcall(testChat.getTabID, testChat)
        m:setRadioChannel(91200); m:setCustomTag("tag"); m:setShouldAttractZombies(false)
        r_chan, r_tag, r_attract = m:getRadioChannel(), m:getCustomTag(), m:isShouldAttractZombies()
        local c = m:clone()
        r_clone = c ~= m and c:getText() == m:getText() and c:getChat() == testChat
        local s = ServerChatMessage.new(testChat, "srv")
        r_srv_author, r_srv_set = s:getAuthor(), pcall(s.setAuthor, s, "x")
        r_srv_isauthor = s:isServerAuthor()
        local n = ChatMessage.new(testChat, "plain")
        n:setAuthor("Zed")
        r_new = n:getAuthor() .. " " .. n:getChatID()
        r_color = m:getTextColor() == testChat:getColor()
        testChat:showMessage("shown", "Bob")
        testChat:showMessage(n)
        r_wrongtab = pcall(showWrongChatTabMessage, 0, 1, "/all")
        r_focus = pcall(focusOnTab, 1)
        r_can_s, r_can_r, r_can_x = checkPlayerCanUseChat(" /s "), checkPlayerCanUseChat("/r"), checkPlayerCanUseChat("/bogus")
        r_pm_self = pcall(proceedPM, "bob hello")
        r_pm1 = pcall(proceedPM, "Ann first")
        r_pm2 = proceedPM("Ann second")
        r_pm_quoted = pcall(proceedPM, '"Ann Lee" hi')
        r_pm_bad = pcall(proceedPM, "x")
        r_faction = pcall(proceedFactionMessage, "f")
        r_admin = pcall(processAdminChatMessage, "a")
        updateChatSettings("small", true, false)
    "#,
    );
    let g = |k: &str| t.globals().get_str(k);
    let s = |k: &str| match g(k) {
        Value::Str(s) => s.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Num(n) => n.to_string(),
        v => format!("{v:?}"),
    };
    // Newest first; "Zed" is no local player, so his message has no bubble.
    assert_eq!(original(&bob, 0, 0), "shown");
    assert_eq!(original(&bob, 0, 1), "hi there");
    assert_eq!(original(&bob, 0, 2), "");
    assert_eq!((s("r_text"), s("r_author")), ("yo <x>".into(), "Bob".into()));
    assert_eq!(s("r_inchat"), "false".to_string());
    assert_eq!(
        s("r_prefix"),
        "<RGB:1.0,1.0,1.0> <SIZE:medium> [13:05][UI_chat_local_chat_title_id][Bob]:  yo &lt;x&gt;"
    );
    assert_eq!(s("r_repl"), "yo &lt;x&gt;");
    assert_eq!(
        (s("r_title"), s("r_type"), s("r_mode")),
        ("UI_chat_local_chat_title_id".into(), "say".into(), "SinglePlayer".into())
    );
    assert_eq!((s("r_range"), s("r_radio"), s("r_tab")), ("30".to_string(), "true".to_string(), "false".to_string()));
    assert_eq!((s("r_chan"), s("r_tag"), s("r_attract")), ("91200".to_string(), "tag".into(), "false".to_string()));
    assert_eq!(s("r_clone"), "true".to_string());
    assert_eq!(
        (s("r_srv_author"), s("r_srv_set"), s("r_srv_isauthor")),
        ("Server".into(), "false".to_string(), "true".to_string())
    );
    assert_eq!(s("r_new"), "Zed -29048394");
    assert_eq!(s("r_color"), "true".to_string());
    assert_eq!((s("r_wrongtab"), s("r_focus")), ("false".to_string(), "false".to_string()));
    assert_eq!(
        (s("r_can_s"), s("r_can_r"), s("r_can_x")),
        ("true".to_string(), "false".to_string(), "false".to_string())
    );
    // Whispers need the server: to yourself fails on the missing server
    // chat, the first to someone else on the missing connection, later
    // ones are queued.
    assert_eq!((s("r_pm_self"), s("r_pm1"), s("r_pm2")), ("false".to_string(), "false".to_string(), "Ann".into()));
    assert_eq!((s("r_pm_quoted"), s("r_pm_bad")), ("false".to_string(), "false".to_string()));
    assert_eq!((s("r_faction"), s("r_admin")), ("false".to_string(), "true".to_string()));
    assert_eq!(crate::bind_core::shared_core().borrow().get_option_chat_font_size(), "small");
    // No single-player chat has a tab, so nothing reaches the chat window.
    assert!(events.borrow().is_empty());

    // With a tab, messages go to Lua's OnAddMessage.
    let general = with_chat_manager(|m| m.chat(ChatType::General)).unwrap();
    general.set_chat_tab(Some(Rc::new(ChatTab::new(0, "UI_chat_main_tab_title_id"))));
    run(&mut t, r#"processGeneralMessage("to all")"#);
    assert_eq!(*events.borrow(), vec!["OnAddMessage 2".to_string()]);
}

#[test]
fn chat_streams_and_types() {
    assert_eq!(
        utility::get_allowed_chat_streams(),
        vec![
            ChatType::Server,
            ChatType::Say,
            ChatType::Radio,
            ChatType::Admin,
            ChatType::Whisper,
            ChatType::Shout,
            ChatType::Safehouse,
            ChatType::Faction,
            ChatType::General
        ]
    );
    assert_eq!(ChatType::from_value(6), ChatType::Radio);
    assert_eq!(ChatType::from_value(42), ChatType::NotDefined);
    assert_eq!(ChatType::NotDefined.value(), -1);
    assert_eq!(ChatType::Server.value(), 8);
    let tab = ChatTab::with_chat(2, "UI_chat_admin_tab_title_id", 5);
    assert!(tab.remove_chat(5).is_ok());
    assert_eq!(tab.remove_chat(5).unwrap_err().message, "Tab '2' doesn't contains a chat id: 5");
}
