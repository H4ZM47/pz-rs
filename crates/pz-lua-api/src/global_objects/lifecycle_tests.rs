//! Authored Lua systems over the real registries, lookup and gos codec.
use super::*;
use crate::{
    bind_core, bind_global_objects,
    events::{EventManager, EVENT_NAMES},
    global_objects_save,
};
use pz_core::core::Core;
use pz_kahlua::stdlib;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    thread: Thread,
    events: EventManager,
    dir: PathBuf,
    old_core: Rc<RefCell<Core>>,
    old_save_dir: Option<PathBuf>,
}

impl Fixture {
    fn new() -> Self {
        let old_core = bind_core::shared_core();
        let old_save_dir = global_objects_save::save_dir();
        bind_core::install_core(Rc::new(RefCell::new(Core::default())));
        reset_single_player_packets();
        reset(Side::Client);
        reset(Side::Server);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = loop {
            let serial = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let candidate = std::env::temp_dir().join(format!(
                "pz-rs-global-lifecycle-{}-{stamp}-{serial}",
                std::process::id()
            ));
            match std::fs::create_dir(&candidate) {
                Ok(()) => break candidate,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create lifecycle fixture: {error}"),
            }
        };
        global_objects_save::set_save_dir(Some(dir.clone()));
        init_lookup(GridBounds {
            min_x: -2,
            min_y: 3,
            width: 2,
            height: 1,
        });
        let mut thread = Thread::new(TableRef::new());
        stdlib::install(&mut thread, None).unwrap();
        let events = EventManager::register(thread.globals(), "Events", EVENT_NAMES);
        bind_global_objects::register_with(&mut thread, events.clone());
        let mut fixture = Self {
            thread,
            events,
            dir,
            old_core,
            old_save_dir,
        };
        fixture.run("function check(value) if not value then error('authored lifecycle check failed') end end");
        fixture
    }

    fn run(&mut self, source: &str) {
        let f = stdlib::load_string(
            source,
            Some("authored-global-lifecycle"),
            self.thread.globals().clone(),
        )
        .unwrap();
        self.thread.pcall(&f, &[]).unwrap();
    }

    fn init(&mut self) {
        init_server_systems(&mut self.thread, &self.events);
        init_client_systems(&mut self.thread, &self.events);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        reset_single_player_packets();
        reset(Side::Client);
        reset(Side::Server);
        global_objects_save::set_save_dir(self.old_save_dir.take());
        bind_core::install_core(self.old_core.clone());
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn global_lifecycle_new_and_saved_systems_keep_real_initial_state_and_lookup() {
    let mut f = Fixture::new();
    f.run(
        r#"
        order = {}
        local serverClass = {}
        serverClass.getInitialStateForClient = function(self)
            retainedInitial = {count=self.count, marker={value='original'}}
            return retainedInitial
        end
        Events.OnSGlobalObjectSystemInit.Add(function()
            table.insert(order, 'server')
            server = SGlobalObjects.registerSystem('authored')
            local data = server:getModData()
            -- Shipped systems inherit native callbacks from their Lua class.
            setmetatable(data, serverClass)
            data.count = data.count or 7
            server:setModDataKeys({'count'})
            server:setObjectModDataKeys({'value'})
            server:setObjectSyncKeys({'value'})
            if server:getObjectCount() == 0 then
                server:newObject(-500, 800, 0):getModData().value = 11
            end
        end)
        Events.OnCGlobalObjectSystemInit.Add(function()
            table.insert(order, 'client')
            client = CGlobalObjects.registerSystem('authored')
            retainedClient = client:getModData()
        end)
    "#,
    );
    f.init();
    f.run(
        r#"
        check(table.concat(order, ',') == 'server,client')
        check(server:loadedWorldVersion() == -1)
        check(retainedClient == client:getModData() and retainedClient.count == 7)
        check(client:getObjectCount() == 1)
        check(client:getObjectAt(-500,800,0):getModData().value == 11)
        check(server:getObjectAt(-500,800,0) == server:getObjectByIndex(0))
        retainedInitial.marker.value = 'server-only mutation'
        check(retainedClient.marker.value == 'original')
        oldClient = retainedClient
        server:getModData().count = 19
        server:getModData().notSaved = 'excluded'
        server:getObjectByIndex(0):getModData().value = 23
    "#,
    );
    global_objects_save::save_all(&f.dir, false, false).unwrap();
    assert!(global_objects_save::file_name(&f.dir, "authored").is_file());
    reset(Side::Client);
    reset(Side::Server);
    init_lookup(GridBounds {
        min_x: -2,
        min_y: 3,
        width: 2,
        height: 1,
    });
    f.run("order = {}");
    f.init();
    f.run(
        r#"
        check(table.concat(order, ',') == 'server,client')
        check(server:loadedWorldVersion() == 249)
        check(server:getObjectCount() == 1 and client:getObjectCount() == 1)
        check(server:getModData().count == 19 and server:getModData().notSaved == nil)
        check(retainedClient ~= oldClient and retainedClient.count == 19)
        check(client:getObjectAt(-500,800,0):getModData().value == 23)
        check(server:getObjectAt(-500,800,0):getModData().value == 23)
    "#,
    );
}

#[test]
fn global_lifecycle_commands_use_inherited_live_class_callbacks() {
    let mut f = Fixture::new();
    f.run(
        r#"
        commands = {}
        local server = SGlobalObjects.registerSystem('inherited')
        local client = CGlobalObjects.registerSystem('inherited')
        local serverClass, clientClass = {}, {}
        setmetatable(server:getModData(), serverClass)
        setmetatable(client:getModData(), clientClass)
        serverClass.OnClientCommand = function(self, command, player, args)
            check(self == server:getModData() and command == 'request' and args.value == 7)
            table.insert(commands, 'server')
            server:sendCommand('reply', {value=args.value})
        end
        clientClass.OnServerCommand = function(self, command, args)
            check(self == client:getModData() and command == 'reply' and args.value == 7)
            table.insert(commands, 'client')
        end
        inheritedClient = client
    "#,
    );
    // Supply an actual player userdata, as the production client protocol
    // requires, instead of bypassing sendCommand's argument conversion.
    let sm = crate::bind_registry::shared_script_manager();
    let player = Rc::new(RefCell::new(
        crate::bind_character_sim::LuaGameCharacter::new(&sm, pz_characters::CharacterKind::Player),
    ));
    f.thread.globals().set_str(
        "packetPlayer",
        crate::bind_character_sim::character_value(&player),
    );
    f.run("inheritedClient:sendCommand('request', packetPlayer, {value=7})");
    bind_core::shared_core().borrow_mut().is_client = true;
    update_server(&mut f.thread);
    update_client(&mut f.thread);
    f.run("check(#commands == 0)");
    bind_core::shared_core().borrow_mut().is_client = false;
    update_server(&mut f.thread);
    update_client(&mut f.thread);
    f.run("check(table.concat(commands, ',') == 'server,client')");
}

#[test]
fn global_lifecycle_network_gates_preserve_original_init_event_order() {
    let mut f = Fixture::new();
    f.run(
        r#"
        serverEvents,clientEvents,handoffs = 0,0,0
        Events.OnSGlobalObjectSystemInit.Add(function()
            serverEvents = serverEvents+1
            local system = SGlobalObjects.registerSystem('network-gate')
            system:getModData().getInitialStateForClient = function(self)
                handoffs = handoffs+1
                return {}
            end
        end)
        Events.OnCGlobalObjectSystemInit.Add(function() clientEvents=clientEvents+1 end)
    "#,
    );
    bind_core::shared_core().borrow_mut().is_client = true;
    f.init();
    f.run("check(serverEvents==0 and clientEvents==1 and handoffs==0)");
    {
        let core = bind_core::shared_core();
        let mut core = core.borrow_mut();
        core.is_client = false;
        core.is_server = true;
    }
    f.init();
    f.run("check(serverEvents==1 and clientEvents==2 and handoffs==0)");
    bind_core::shared_core().borrow_mut().is_server = false;
    f.init();
    f.run("check(serverEvents==2 and clientEvents==3 and handoffs==1)");
}

#[test]
fn global_lifecycle_packet_reset_retires_commands_before_a_new_session() {
    let mut f = Fixture::new();
    f.run(
        r#"
        server = SGlobalObjects.registerSystem('retired')
        server:sendCommand('old-world', {value=1})
    "#,
    );
    reset_single_player_packets();
    reset(Side::Client);
    reset(Side::Server);
    init_lookup(GridBounds {
        min_x: -2,
        min_y: 3,
        width: 2,
        height: 1,
    });
    f.run(
        r#"
        received=0
        local client = CGlobalObjects.registerSystem('retired')
        client:getModData().OnServerCommand = function(self, command, args)
            received=received+1
        end
    "#,
    );
    update_client(&mut f.thread);
    f.run("check(received==0)");
}

#[test]
fn global_lifecycle_client_registry_reset_preserves_transport_packets() {
    let mut f = Fixture::new();
    f.run(
        r#"
        local server = SGlobalObjects.registerSystem('registry-only')
        CGlobalObjects.registerSystem('registry-only')
        server:sendCommand('queued', {value=9})
        CGlobalObjects.Reset()
        received = 0
        local replacement = CGlobalObjects.registerSystem('registry-only')
        replacement:getModData().OnServerCommand = function(self, command, args)
            check(command == 'queued' and args.value == 9)
            received = received + 1
        end
    "#,
    );
    update_client(&mut f.thread);
    f.run("check(received == 1)");
}
