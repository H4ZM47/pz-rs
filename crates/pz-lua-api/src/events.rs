//! `LuaEventManager` and `LuaHookManager`: the `Events` and `Hook`
//! tables scripts register callbacks on, and their triggering.

use pz_kahlua::vm::LuaClosure;
use pz_kahlua::gc::{traceable, TracePayload, TraceVisitor};
use pz_kahlua::{LuaResult, NativeCall, NativeFunction, TableRef, Thread, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Events registered at startup by `LuaEventManager.AddEvents`, in order.
pub const EVENT_NAMES: &[&str] = &[
    "OnGameBoot",
    "OnPreGameStart",
    "OnTick",
    "OnTickEvenPaused",
    "OnRenderUpdate",
    "OnFETick",
    "OnGameStart",
    "OnPreUIDraw",
    "OnPostUIDraw",
    "OnCharacterCollide",
    "OnKeyStartPressed",
    "OnKeyPressed",
    "OnContextKey",
    "OnObjectCollide",
    "OnNPCSurvivorUpdate",
    "OnPlayerUpdate",
    "OnZombieUpdate",
    "OnZombieCreate",
    "OnTriggerNPCEvent",
    "OnMultiTriggerNPCEvent",
    "OnLoadMapZones",
    "OnLoadedMapZones",
    "OnAddBuilding",
    "OnCreateLivingCharacter",
    "OnChallengeQuery",
    "OnClickedAnimalForContext",
    "OnFillInventoryObjectContextMenu",
    "OnPreFillInventoryObjectContextMenu",
    "OnFillWorldObjectContextMenu",
    "OnPreFillWorldObjectContextMenu",
    "OnRefreshInventoryWindowContainers",
    "OnGamepadConnect",
    "OnGamepadDisconnect",
    "OnJoypadActivate",
    "OnJoypadActivateUI",
    "OnJoypadBeforeDeactivate",
    "OnJoypadDeactivate",
    "OnJoypadBeforeReactivate",
    "OnJoypadReactivate",
    "OnJoypadRenderUI",
    "OnJoypadDebugRenderUIOptionSet",
    "OnMakeItem",
    "OnWeaponHitCharacter",
    "OnWeaponSwing",
    "OnWeaponHitTree",
    "OnWeaponHitXp",
    "OnWeaponSwingHitPoint",
    "OnPlayerAttackFinished",
    "OnLoginState",
    "OnLoginStateSuccess",
    "OnCharacterCreateStats",
    "OnLoadSoundBanks",
    "OnObjectLeftMouseButtonDown",
    "OnObjectLeftMouseButtonUp",
    "OnObjectRightMouseButtonDown",
    "OnObjectRightMouseButtonUp",
    "OnDoTileBuilding",
    "OnDoTileBuilding2",
    "OnDoTileBuilding3",
    "RenderOpaqueObjectsInWorld",
    "OnConnectFailed",
    "OnConnected",
    "OnDisconnect",
    "OnConnectionStateChanged",
    "OnQRReceived",
    "OnGoogleAuthRequest",
    "OnScoreboardUpdate",
    "OnMouseMove",
    "OnMouseDown",
    "OnMouseUp",
    "OnRightMouseDown",
    "OnRightMouseUp",
    "OnMouseWheel",
    "OnNewSurvivorGroup",
    "OnPlayerSetSafehouse",
    "OnLoad",
    "AddXP",
    "LevelPerk",
    "LogLevelPerk",
    "OnSave",
    "OnMainMenuEnter",
    "OnGameStateEnter",
    "OnPreMapLoad",
    "OnPostFloorSquareDraw",
    "OnPostFloorLayerDraw",
    "OnPostTilesSquareDraw",
    "OnPostTileDraw",
    "OnPostWallSquareDraw",
    "OnPostCharactersSquareDraw",
    "OnCreateUI",
    "OnMapLoadCreateIsoObject",
    "OnCreateSurvivor",
    "OnCreatePlayer",
    "OnPlayerDeath",
    "OnZombieDead",
    "OnCharacterDeath",
    "OnCharacterMeet",
    "OnSpawnRegionsLoaded",
    "OnPostMapLoad",
    "OnAIStateExecute",
    "OnAIStateEnter",
    "OnAIStateExit",
    "OnAIStateChange",
    "OnPlayerMove",
    "OnInitWorld",
    "OnNewGame",
    "OnIsoThumpableLoad",
    "OnIsoThumpableSave",
    "ReuseGridsquare",
    "LoadGridsquare",
    "LoadChunk",
    "EveryOneMinute",
    "EveryTenMinutes",
    "EveryDays",
    "EveryHours",
    "OnDusk",
    "OnDawn",
    "OnEquipPrimary",
    "OnEquipSecondary",
    "OnClothingUpdated",
    "OnWeatherPeriodStart",
    "OnWeatherPeriodStage",
    "OnWeatherPeriodComplete",
    "OnWeatherPeriodStop",
    "OnRainStart",
    "OnRainStop",
    "OnAmbientSound",
    "OnWorldSound",
    "OnResetLua",
    "OnModsModified",
    "OnSeeNewRoom",
    "OnNewFire",
    "OnFillContainer",
    "OnChangeWeather",
    "OnRenderTick",
    "OnDestroyIsoThumpable",
    "OnPostSave",
    "OnResolutionChange",
    "OnWaterAmountChange",
    "OnClientCommand",
    "OnServerCommand",
    "OnProcessTransaction",
    "OnProcessAction",
    "OnContainerUpdate",
    "OnObjectAdded",
    "OnObjectAboutToBeRemoved",
    "onLoadModDataFromServer",
    "OnGameTimeLoaded",
    "OnCGlobalObjectSystemInit",
    "OnSGlobalObjectSystemInit",
    "OnWorldMessage",
    "OnKeyKeepPressed",
    "SendCustomModData",
    "ServerPinged",
    "OnServerStarted",
    "OnLoadedTileDefinitions",
    "OnPostRender",
    "DoSpecialTooltip",
    "OnCoopJoinFailed",
    "OnServerWorkshopItems",
    "OnVehicleDamageTexture",
    "OnCustomUIKey",
    "OnCustomUIKeyPressed",
    "OnCustomUIKeyReleased",
    "OnTileObjectAdded",
    "OnDeviceText",
    "OnRadioInteraction",
    "OnLoadRadioScripts",
    "OnAcceptInvite",
    "OnCoopServerMessage",
    "OnReceiveUserlog",
    "OnAdminMessage",
    "ReceiveFactionInvite",
    "AcceptedFactionInvite",
    "ReceiveSafehouseInvite",
    "AcceptedSafehouseInvite",
    "ViewTickets",
    "ViewBannedIPs",
    "ViewBannedSteamIDs",
    "SyncFaction",
    "SyncFactionServer",
    "RefreshCheats",
    "OnReceiveItemListNet",
    "OnMiniScoreboardUpdate",
    "OnSafehousesChanged",
    "OnWarUpdate",
    "RequestTrade",
    "AcceptedTrade",
    "RequestMedicalCheck",
    "AcceptedMedicalCheck",
    "TradingUIAddItem",
    "TradingUIRemoveItem",
    "TradingUIUpdateState",
    "OnGridBurnt",
    "OnPreDistributionMerge",
    "OnDistributionMerge",
    "OnPostDistributionMerge",
    "MngInvReceiveItems",
    "OnTileRemoved",
    "OnServerStartSaving",
    "OnServerFinishSaving",
    "OnMechanicActionDone",
    "OnClimateTick",
    "OnThunderEvent",
    "OnEnterVehicle",
    "OnSteamGameJoin",
    "OnTabAdded",
    "OnSetDefaultTab",
    "OnTabRemoved",
    "OnAddMessage",
    "SwitchChatStream",
    "OnChatWindowInit",
    "OnAlertMessage",
    "OnInitSeasons",
    "OnClimateTickDebug",
    "OnInitModdedWeatherStage",
    "OnUpdateModdedWeatherStage",
    "OnClimateManagerInit",
    "OnPressReloadButton",
    "OnPressRackButton",
    "OnPressWalkTo",
    "OnHitZombie",
    "OnBeingHitByZombie",
    "OnServerStatisticReceived",
    "OnDynamicMovableRecipe",
    "OnInitGlobalModData",
    "OnReceiveGlobalModData",
    "OnInitRecordedMedia",
    "onUpdateIcon",
    "preAddForageDefs",
    "preAddSkillDefs",
    "preAddZoneDefs",
    "preAddCatDefs",
    "preAddItemDefs",
    "onAddForageDefs",
    "onFillSearchIconContextMenu",
    "onItemFall",
    "OnTemplateTextInit",
    "OnPlayerGetDamage",
    "OnWeaponHitThumpable",
    "OnFishingActionMPUpdate",
    "OnThrowableExplode",
    "OnSourceWindowFileReload",
    "OnSpawnVehicleStart",
    "OnSpawnVehicleEnd",
    "OnMovingObjectCrop",
    "OnOverrideSearchManager",
    "OnSleepingTick",
    "OnRolesReceived",
    "OnNetworkUsersReceived",
    "OnServerCustomizationDataReceived",
    "OnDeadBodySpawn",
    "OnAnimalTracks",
    "OnItemFound",
    "SetDragItem",
    "OnSteamServerResponded",
    "OnSteamServerResponded2",
    "OnSteamServerFailedToRespond2",
    "OnSteamRulesRefreshComplete",
    "OnSteamRefreshInternetServers",
    "OptionControllerButtonStyleChanged",
    "OptionGamepadBindingPresetChanged",
];

/// Hooks registered by `LuaHookManager.AddEvents`.
pub const HOOK_NAMES: &[&str] = &[
    "AutoDrink",
    "UseItem",
    "Attack",
    "CalculateStats",
    "ContextualAction",
    "WeaponHitCharacter",
    "WeaponSwing",
    "WeaponSwingHitPoint",
];

#[derive(Default)]
pub struct Event {
    pub name: String,
    pub callbacks: Vec<Rc<LuaClosure>>,
}

impl TracePayload for Event {
    fn trace(&self, visitor: &mut TraceVisitor) {
        for callback in &self.callbacks {
            visitor.closure(callback);
        }
    }

    fn detach(&mut self) -> Option<Box<dyn std::any::Any>> {
        Some(Box::new(std::mem::take(&mut self.callbacks)))
    }
}

// Each Add/Remove callable owns one capture backing allocation, whose
// physical Event Rc slot is counted independently of any other callable.
struct EventCapture {
    event: Rc<RefCell<Event>>,
}

impl TracePayload for EventCapture {
    fn trace(&self, visitor: &mut TraceVisitor) {
        visitor.owner(&self.event);
    }
}

fn add_callback(capture: &RefCell<EventCapture>, c: &mut NativeCall<'_>) -> LuaResult<usize> {
    let event = capture.borrow().event.clone();
    if let Value::Closure(callback) = c.arg(0) {
        event.borrow_mut().callbacks.push(callback);
    }
    Ok(0)
}

fn remove_callback(capture: &RefCell<EventCapture>, c: &mut NativeCall<'_>) -> LuaResult<usize> {
    let event = capture.borrow().event.clone();
    if let Value::Closure(callback) = c.arg(0) {
        let mut event = event.borrow_mut();
        // ArrayList.remove(Object): first equal element.
        if let Some(index) = event.callbacks.iter().position(|value| Rc::ptr_eq(value, &callback)) {
            event.callbacks.remove(index);
        }
    }
    Ok(0)
}

/// One event table family (`Events` or `Hook`).
#[derive(Clone)]
pub struct EventManager {
    inner: Rc<RefCell<Registry>>,
    table: TableRef,
}

#[derive(Default)]
struct Registry {
    list: Vec<Rc<RefCell<Event>>>,
    map: HashMap<String, Rc<RefCell<Event>>>,
}

impl EventManager {
    /// `register(platform, env)`: create `env[global]` and add `names`.
    pub fn register(env: &TableRef, global: &str, names: &[&str]) -> Self {
        let table = TableRef::new();
        env.set_str(global, table.clone());
        let m = EventManager {
            inner: Rc::default(),
            table,
        };
        for n in names {
            m.add_event(n);
        }
        m
    }

    /// `AddEvent(name)`.
    pub fn add_event(&self, name: &str) -> Rc<RefCell<Event>> {
        if let Some(e) = self.inner.borrow().map.get(name) {
            return e.clone();
        }
        let e = traceable(Event {
            name: name.to_string(),
            callbacks: Vec::new(),
        });
        {
            let mut r = self.inner.borrow_mut();
            r.list.push(e.clone());
            r.map.insert(name.to_string(), e.clone());
        }
        let t = TableRef::new();
        t.set_str(
            "Add",
            Value::Native(NativeFunction::new_traced_capture("Add", EventCapture { event: e.clone() }, add_callback)),
        );
        t.set_str(
            "Remove",
            Value::Native(NativeFunction::new_traced_capture("Remove", EventCapture { event: e.clone() }, remove_callback)),
        );
        self.table.set_str(name, t);
        e
    }

    /// `triggerEvent(name, params...)`. Unknown events are added (with a
    /// log line) like the original's `checkEvent`.
    pub fn trigger(&self, thread: &mut Thread, name: &str, params: &[Value]) -> bool {
        let e = match self.inner.borrow().map.get(name).cloned() {
            Some(e) => e,
            None => {
                (thread.out)(&format!("LuaEventManager: adding unknown event \"{name}\""));
                drop(self.add_event(name));
                return false;
            }
        };
        trigger_registered_event(thread, &e, params)
    }

    pub fn callbacks(&self, name: &str) -> usize {
        self.inner
            .borrow()
            .map
            .get(name)
            .map_or(0, |e| e.borrow().callbacks.len())
    }
}

/// Trigger an already registered event without capturing its entire registry
/// in a native binding. The caller can trace this event's actual owning slot.
pub(crate) fn trigger_registered_event(
    thread: &mut Thread,
    e: &Rc<RefCell<Event>>,
    params: &[Value],
) -> bool {
    if e.borrow().callbacks.is_empty() {
        return false;
    }
    // Event.trigger: index-based loop that tolerates callbacks removing
    // themselves during the call.
    let mut n = 0;
    loop {
        let cb = match e.borrow().callbacks.get(n) {
            Some(cb) => cb.clone(),
            None => break,
        };
        if let Err((msg, trace)) = thread.pcall(&Value::Closure(cb.clone()), params) {
            (thread.log)(&format!(
                "{}\n{}",
                pz_kahlua::exec::java_value_of(&msg),
                trace
            ));
        }
        if !e.borrow().callbacks.iter().any(|x| Rc::ptr_eq(x, &cb)) {
            continue;
        }
        n += 1;
    }
    true
}

#[cfg(test)]
mod gc_tests;
