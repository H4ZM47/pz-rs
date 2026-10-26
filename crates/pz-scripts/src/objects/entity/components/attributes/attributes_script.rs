//! `zombie.scripting.entity.components.attributes.AttributesScript` and the
//! attribute names `zombie.entity.components.attributes.Attribute` registers.

use crate::base::ScriptResult;
use crate::java::{self, JHashMap};
use crate::manager::ScriptManager;
use crate::objects::entity::component_script::cast;
use crate::objects::entity::{log_error, ComponentBase, ComponentScript, ComponentType};
use crate::parser::Block;
use std::any::Any;
use std::cell::{BorrowError, BorrowMutError, Cell, Ref, RefCell};
use std::fmt;
use std::rc::Rc;

/// The actual runtime AttributeContainer component allocation. The scripts
/// crate cannot depend on the runtime entity crate, so only its Rc is erased.
/// This is the native container field, not a copied template data body.
pub type AttributeTemplate = Rc<dyn Any>;

/// Runtime construction only. The provider must use the same actual lazy
/// builder as the native getter, queue diagnostics for an outer safe point,
/// and mark a completed native attempt even when its result is null.
pub type AttributeTemplateCreator = fn(&AttributesScript) -> ScriptResult<()>;

thread_local! {
    static TEMPLATE_CREATOR: Cell<Option<AttributeTemplateCreator>> = const { Cell::new(None) };
    // Old cache fields awaiting an outer host drop. This queue is retirement
    // bookkeeping, never a second live template lookup or cache authority.
    static RETIRED_TEMPLATES: RefCell<Vec<AttributeTemplate>> = const { RefCell::new(Vec::new()) };
}

pub fn install_attribute_template_creator(
    creator: Option<AttributeTemplateCreator>,
) -> Option<AttributeTemplateCreator> {
    TEMPLATE_CREATOR.with(|slot| slot.replace(creator))
}

/// Move old cache pins out of script reload scopes. Drop the returned owners
/// only after every script, entity, world and sprite guard has ended.
pub fn take_retired_attribute_templates() -> Vec<AttributeTemplate> {
    RETIRED_TEMPLATES.with(|queue| std::mem::take(&mut *queue.borrow_mut()))
}

fn retire_attribute_template(template: AttributeTemplate) {
    let mut owner = Some(template);
    let _ = RETIRED_TEMPLATES.try_with(|queue| {
        if let Ok(mut queue) = queue.try_borrow_mut() {
            queue.push(owner.take().expect("retirement owns its cache pin"));
        }
    });
    // During TLS teardown, or an unavailable enclosing retirement borrow,
    // releasing a final native payload here could reenter dead/borrowed host
    // state. Preserve that pin conservatively instead of dropping it here.
    if let Some(owner) = owner {
        std::mem::forget(owner);
    }
}

/// Names of the attribute types `Attribute` registers statically.
/// TODO(AttributeType): value types, ids and bounds live with the runtime
/// attribute system.
const ATTRIBUTE_TYPE_NAMES: &[&str] = &[
    "TestString",
    "TestString2",
    "TestQuality",
    "TestCondition",
    "TestBool",
    "TestUses",
    "TestItemType",
    "TestCategories",
    "TestTags",
    "Sharpness",
    "HeadCondition",
    "HeadConditionMax",
    "Quality",
    "TimesHeadRepaired",
    "OriginX",
    "OriginY",
    "OriginZ",
];

/// `Attribute.TypeFromName`: case-insensitive lookup; returns the
/// registered name.
pub fn attribute_type_from_name(name: &str) -> Option<&'static str> {
    let lower = java::to_lower(name);
    ATTRIBUTE_TYPE_NAMES
        .iter()
        .copied()
        .find(|n| java::to_lower(n) == lower)
}

pub struct AttributesScript {
    pub component: ComponentBase,
    kv_pairs: JHashMap<String>,
    has_created_container: Cell<bool>,
    template: RefCell<Option<AttributeTemplate>>,
}

impl fmt::Debug for AttributesScript {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AttributesScript")
            .field("component", &self.component)
            .field("kv_pairs", &self.kv_pairs)
            .field("has_created_container", &self.has_created_container.get())
            .field(
                "template",
                &self
                    .template
                    .try_borrow()
                    .map(|slot| slot.as_ref().map(|owner| Rc::as_ptr(owner) as *const ())),
            )
            .finish()
    }
}

impl Drop for AttributesScript {
    fn drop(&mut self) {
        // Parent script reload/removal can drop this Box without calling the
        // native PreReload hook. Only move the remaining actual cache field;
        // this is no native reset, release or class-pool operation.
        if let Some(old) = self.template.get_mut().take() {
            retire_attribute_template(old);
        }
    }
}

impl Default for AttributesScript {
    fn default() -> Self {
        Self::new()
    }
}

impl AttributesScript {
    pub fn new() -> Self {
        AttributesScript {
            component: ComponentBase::new(ComponentType::Attributes),
            kv_pairs: JHashMap::new(),
            has_created_container: Cell::new(false),
            template: RefCell::new(None),
        }
    }

    /// The `kvPairs` map: attribute key as written to its value.
    pub fn kv_pairs(&self) -> &JHashMap<String> {
        &self.kv_pairs
    }

    pub fn template_created(&self) -> bool {
        self.has_created_container.get()
    }

    /// Borrow the ONE physical native cache field without cloning the owner.
    /// Metadata tracing uses this view; runtime getters clone this same Rc.
    pub fn template_slot(&self) -> Result<Ref<'_, Option<AttributeTemplate>>, BorrowError> {
        self.template.try_borrow()
    }

    /// Native creation stores the obtained component before parsing values.
    /// Return the former owning slot for outer-safe retirement. Borrowing the
    /// incoming owner ensures a rejected write cannot drop its last pin here.
    pub fn replace_template(
        &self,
        template: Option<&AttributeTemplate>,
    ) -> Result<Option<AttributeTemplate>, BorrowMutError> {
        let mut slot = self.template.try_borrow_mut()?;
        let old = std::mem::replace(&mut *slot, template.cloned());
        drop(slot);
        Ok(old)
    }

    /// Native PC162 runs after a successful, empty, or caught-error attempt.
    /// Unsupported host capability failures must not call this setter.
    pub fn mark_template_created(&self) {
        self.has_created_container.set(true);
    }

    pub fn publish_template(
        &self,
        template: Option<&AttributeTemplate>,
    ) -> Result<Option<AttributeTemplate>, BorrowMutError> {
        let old = self.replace_template(template)?;
        self.mark_template_created();
        Ok(old)
    }

    /// Actual `createTemplateContainer`, shared by eager script loading and
    /// the lazy getter. The runtime provider owns native allocation, parsing,
    /// diagnostic and exception semantics; unavailable support is explicit.
    pub fn create_template_container(&self) -> ScriptResult<()> {
        if self.template_created() {
            return Ok(());
        }
        if self.kv_pairs.is_empty() {
            self.mark_template_created();
            return Ok(());
        }
        let creator = TEMPLATE_CREATOR.with(Cell::get).ok_or_else(|| {
            crate::objects::entity::runtime_exception(
                "AttributesScript template runtime provider is unavailable",
            )
        })?;
        creator(self)?;
        if !self.template_created() {
            return Err(crate::objects::entity::runtime_exception(
                "AttributesScript template runtime provider did not complete its native attempt",
            ));
        }
        Ok(())
    }

    pub fn get_template_container(&self) -> ScriptResult<Option<AttributeTemplate>> {
        self.create_template_container()?;
        let slot = self.template_slot().map_err(|_| {
            crate::objects::entity::runtime_exception("AttributesScript template field is borrowed")
        })?;
        Ok(slot.as_ref().cloned())
    }
}

impl ComponentScript for AttributesScript {
    component_accessors!("zombie.scripting.entity.components.attributes.AttributesScript");

    fn pre_reload(&mut self) {
        self.kv_pairs.clear();
        let old = self.template.get_mut().take();
        self.has_created_container.set(false);
        if let Some(old) = old {
            retire_attribute_template(old);
        }
    }

    fn on_scripts_loaded(
        &mut self,
        _sm: &ScriptManager,
        _mode: crate::script_type::ScriptLoadMode,
    ) -> ScriptResult {
        self.create_template_container()
    }

    /// `copyFrom`: `kvPairs.putAll(other.kvPairs)`.
    fn copy_from(&mut self, other: &dyn ComponentScript) -> ScriptResult {
        let other = cast::<AttributesScript>(other, self)?;
        for (k, v) in other.kv_pairs.iter() {
            self.kv_pairs.insert(k.to_string(), v.clone());
        }
        Ok(())
    }

    fn load(&mut self, sm: &ScriptManager, block: &Block) -> ScriptResult {
        for value in &block.values {
            let k = java::trim(value.get_key());
            let v = java::trim(value.get_value());
            self.parse_key_value(sm, k, v)?;
        }
        Ok(())
    }

    fn parse_key_value(&mut self, sm: &ScriptManager, k: &str, v: &str) -> ScriptResult<bool> {
        if attribute_type_from_name(k).is_some() {
            self.kv_pairs.insert(k.to_string(), v.to_string());
            return Ok(true);
        }
        log_error(sm, &format!("Unknown attribute, key = {k}, value = {v}"));
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        static CREATIONS: Cell<usize> = const { Cell::new(0) };
    }

    struct Restore(Option<AttributeTemplateCreator>);
    impl Restore {
        fn new(creator: Option<AttributeTemplateCreator>) -> Self {
            drop(take_retired_attribute_templates());
            CREATIONS.with(|count| count.set(0));
            Self(install_attribute_template_creator(creator))
        }
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            install_attribute_template_creator(self.0);
            drop(take_retired_attribute_templates());
        }
    }

    fn nonempty() -> AttributesScript {
        let mut script = AttributesScript::new();
        script
            .parse_key_value(&ScriptManager::new(), "Sharpness", "1.0")
            .unwrap();
        script
    }

    // The scripts crate intentionally has no runtime entity dependency. This
    // payload tests the erased physical slot; actual ComponentRef allocation,
    // shared class pool and independent attribute copies are entity/host tests.
    fn create_actual_payload(script: &AttributesScript) -> ScriptResult<()> {
        CREATIONS.with(|count| count.set(count.get() + 1));
        let actual: AttributeTemplate = Rc::new(RefCell::new(vec![17i32]));
        assert!(script.replace_template(Some(&actual)).unwrap().is_none());
        assert!(!script.template_created());
        script.mark_template_created();
        Ok(())
    }

    fn finish_native_null_attempt(script: &AttributesScript) -> ScriptResult<()> {
        CREATIONS.with(|count| count.set(count.get() + 1));
        assert!(script.publish_template(None).unwrap().is_none());
        Ok(())
    }

    #[test]
    fn empty_template_is_a_completed_native_null_without_a_runtime_provider() {
        let _restore = Restore::new(None);
        let mut script = AttributesScript::new();
        script
            .on_scripts_loaded(&ScriptManager::new(), crate::ScriptLoadMode::Init)
            .unwrap();
        assert!(script.template_created());
        assert!(script.get_template_container().unwrap().is_none());
    }

    #[test]
    fn nonempty_template_without_provider_errors_without_fabricating_created_null() {
        let _restore = Restore::new(None);
        let script = nonempty();
        assert!(script.create_template_container().is_err());
        assert!(!script.template_created());
        assert!(script.template_slot().unwrap().is_none());
        assert!(script.get_template_container().is_err());
    }

    #[test]
    fn eager_creation_and_lazy_getter_retain_one_actual_cache_allocation() {
        let _restore = Restore::new(Some(create_actual_payload));
        let mut script = nonempty();
        script
            .on_scripts_loaded(&ScriptManager::new(), crate::ScriptLoadMode::Init)
            .unwrap();
        let first = script.get_template_container().unwrap().unwrap();
        let payload = Rc::clone(&first)
            .downcast::<RefCell<Vec<i32>>>()
            .ok()
            .expect("the native cache retains the supplied actual allocation");
        payload.borrow_mut().push(23);
        let second = script.get_template_container().unwrap().unwrap();
        assert!(Rc::ptr_eq(&first, &second));
        assert_eq!(*payload.borrow(), [17, 23]);
        assert!(Rc::ptr_eq(
            script.template_slot().unwrap().as_ref().unwrap(),
            &first
        ));
        CREATIONS.with(|count| assert_eq!(count.get(), 1));
    }

    #[test]
    fn completed_native_null_attempt_is_not_rebuilt_by_the_lazy_getter() {
        let _restore = Restore::new(Some(finish_native_null_attempt));
        let script = nonempty();
        assert!(script.get_template_container().unwrap().is_none());
        assert!(script.get_template_container().unwrap().is_none());
        assert!(script.template_created());
        CREATIONS.with(|count| assert_eq!(count.get(), 1));
    }

    #[test]
    fn borrowed_cache_rejects_publication_without_consuming_incoming_owner_or_flag() {
        let _restore = Restore::new(None);
        let script = nonempty();
        let actual: AttributeTemplate = Rc::new(RefCell::new(vec![17i32]));
        let slot = script.template_slot().unwrap();
        assert!(script.publish_template(Some(&actual)).is_err());
        assert_eq!(Rc::strong_count(&actual), 1);
        assert!(!script.template_created());
        assert!(slot.is_none());
        drop(slot);
        assert!(script.publish_template(Some(&actual)).unwrap().is_none());
        assert_eq!(Rc::strong_count(&actual), 2);
        assert!(script.template_created());
    }

    struct RetiredPayload(Rc<RefCell<usize>>);
    impl Drop for RetiredPayload {
        fn drop(&mut self) {
            *self.0.borrow_mut() += 1;
        }
    }

    #[test]
    fn pre_reload_moves_old_cache_pin_without_dropping_it_under_the_script_scope() {
        let _restore = Restore::new(None);
        let count = Rc::new(RefCell::new(0));
        let actual: AttributeTemplate = Rc::new(RefCell::new(RetiredPayload(count.clone())));
        let mut script = nonempty();
        script.publish_template(Some(&actual)).unwrap();
        drop(actual);
        let guard = count.borrow_mut();
        script.pre_reload();
        assert!(script.kv_pairs().is_empty());
        assert!(script.template_slot().unwrap().is_none());
        assert!(!script.template_created());
        assert_eq!(*guard, 0);
        // Dropping a child already cleared by PreReload must not enqueue a
        // duplicate owner or run the final payload under this guard.
        drop(script);
        assert_eq!(*guard, 0);
        drop(guard);
        let retired = take_retired_attribute_templates();
        assert_eq!(retired.len(), 1);
        drop(retired);
        assert_eq!(*count.borrow(), 1);
    }

    #[test]
    fn dropping_a_boxed_script_defers_its_actual_cache_until_enclosing_guards_end() {
        let _restore = Restore::new(None);
        let count = Rc::new(RefCell::new(0));
        let actual: AttributeTemplate = Rc::new(RefCell::new(RetiredPayload(count.clone())));
        let script = Box::new(nonempty());
        script.publish_template(Some(&actual)).unwrap();
        drop(actual);
        let guard = count.borrow_mut();
        drop(script);
        assert_eq!(*guard, 0);
        drop(guard);
        let retired = take_retired_attribute_templates();
        assert_eq!(retired.len(), 1);
        drop(retired);
        assert_eq!(*count.borrow(), 1);
    }

    #[test]
    fn native_copy_from_changes_kv_pairs_without_copying_or_replacing_the_template() {
        let _restore = Restore::new(None);
        let mut destination = AttributesScript::new();
        let actual: AttributeTemplate = Rc::new(RefCell::new(vec![31i32]));
        destination.publish_template(Some(&actual)).unwrap();
        destination.copy_from(&nonempty()).unwrap();
        assert!(destination.template_created());
        assert_eq!(destination.kv_pairs().len(), 1);
        assert!(Rc::ptr_eq(
            destination.template_slot().unwrap().as_ref().unwrap(),
            &actual
        ));
    }
}
