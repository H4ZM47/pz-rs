//! Actual pre-world floor receiver for GameEntity.loadEntity. This supports
//! the genuine Fluid codec; other world component scripts need their own host.
use crate::IsoObjectRef;
use pz_entity::runtime_load::{
    HostResult, NativeComponentIssue, RuntimeHostError, RuntimeLoadHost,
};
use pz_entity::{ComponentRef, ComponentType, EntityContext, RetiredComponents};

pub struct FloorLoadHost<'a> {
    pub object: &'a IsoObjectRef,
    pub context: &'a dyn EntityContext,
}
impl RuntimeLoadHost for FloorLoadHost<'_> {
    fn debug_enabled(&self) -> HostResult<bool> {
        pz_inventory::component_host::component_debug_enabled()
    }
    fn resolve_type(&mut self, id: i16) -> HostResult<ComponentType> {
        if !(0..=26).contains(&id) {
            return Err(RuntimeHostError::NativeException(pz_entity::EntityError(
                "ArrayIndexOutOfBoundsException".into(),
            )));
        }
        Ok(ComponentType::from_id(id))
    }
    fn get_component(&mut self, kind: ComponentType) -> HostResult<Option<ComponentRef>> {
        self.object
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("floor load component lookup"))?
            .components
            .get_ref(kind)
            .map_err(RuntimeHostError::Storage)
    }
    fn component_size(&mut self) -> HostResult<usize> {
        Ok(self
            .object
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("floor load component size"))?
            .components
            .len())
    }
    fn component_at(&mut self, index: usize) -> HostResult<Option<ComponentRef>> {
        self.object
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("floor load component index"))?
            .components
            .get_ref_at(index)
            .map_err(RuntimeHostError::Storage)
    }
    fn obtain_component(
        &mut self,
        kind: ComponentType,
        _: &mut RetiredComponents,
    ) -> HostResult<Option<ComponentRef>> {
        pz_entity::runtime_pool::obtain_component(kind)
            .map(Some)
            .map_err(pz_inventory::component_host::pool_error)
    }
    fn load_component(
        &mut self,
        component: &ComponentRef,
        reader: &mut pz_entity::io::Reader<'_>,
        version: i32,
        retired: &mut RetiredComponents,
    ) -> HostResult<()> {
        let outcome = {
            let mut body = component
                .try_borrow_mut()
                .map_err(|_| RuntimeHostError::Borrowed("floor load fluid body"))?;
            if body.kind() != ComponentType::FluidContainer {
                return Err(RuntimeHostError::Unsupported {
                    operation: "actual world component codec beyond FluidContainer",
                    kind: Some(body.kind()),
                });
            }
            let native = |error| match error {
                pz_entity::RuntimeError::Codec(error) => RuntimeHostError::NativeException(error),
                error => RuntimeHostError::Storage(error),
            };
            body.load_payload_in_place(reader, version, self.context, retired)
                .map_err(native)?;
            body.fluid_recalculate_data(self.context, true)
                .map_err(native)?
        };
        let result = pz_inventory::component_host::notify_fluid_update(&outcome);
        if let Some(owner) = outcome.owner {
            retired.push_owner(owner);
        }
        result
    }
    fn add_component(
        &mut self,
        component: &ComponentRef,
        retired: &mut RetiredComponents,
    ) -> HostResult<bool> {
        super::runtime::add_before_world(self.object, component, retired)
    }
    fn connect_components(&mut self, _: &mut RetiredComponents) -> HostResult<()> {
        super::runtime::connect_world(self.object)
    }
    fn remove_component_exact(
        &mut self,
        component: &ComponentRef,
        retired: &mut RetiredComponents,
    ) -> HostResult<Option<ComponentRef>> {
        super::runtime::remove_world_component(self.object, component, retired)
    }
    fn release_component(
        &mut self,
        component: &ComponentRef,
        retired: &mut RetiredComponents,
    ) -> HostResult<()> {
        super::runtime::release_detached(component, retired)
    }
    fn log_exception(&mut self, issue: &NativeComponentIssue) -> HostResult<()> {
        pz_inventory::component_host::emit_native_notices(&[
            pz_entity::runtime_fluid::RuntimeNativeNotice::ExceptionLogger(
                issue.exception.to_string(),
            ),
        ])
    }
}
