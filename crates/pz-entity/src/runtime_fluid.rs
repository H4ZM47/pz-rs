//! The actual nonserialized cache allocations owned by a live FluidContainer.
//!
//! `color` and `propertiesCache` are final native objects. Reset changes their
//! contents, and a retained handle keeps observing those same allocations.
//! The legacy FluidContainer value remains the sole saved/script data body.

use std::cell::RefCell;
use std::rc::Rc;

use crate::color::Rgba;
use crate::fluids::{FluidContainer, SealedFluidProperties};
use crate::{EntityContext, EntityOwner, Lookup, RuntimeError};

pub type FluidColorRef = Rc<RefCell<Rgba>>;
pub type FluidPropertiesRef = Rc<RefCell<SealedFluidProperties>>;

/// An owned native diagnostic for the host to emit after all component,
/// script and entity guards end. Callers retain notices across a later error
/// so an already completed native logging prefix is not silently lost.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeNativeNotice {
    GeneralLog(String),
    GeneralError(String),
    GeneralException(String),
    ExceptionLogger(String),
}

thread_local! {
    // FluidContainer.colorDef aliases the actual mutable Color.white object.
    static FLUID_DEFAULT_COLOR: FluidColorRef = Rc::new(RefCell::new([1.0; 4]));
}

pub fn fluid_default_color_slot() -> FluidColorRef {
    FLUID_DEFAULT_COLOR.with(Rc::clone)
}

pub(crate) fn copy_default_color() -> Result<Rgba, RuntimeError> {
    FLUID_DEFAULT_COLOR.with(|color| {
        color
            .try_borrow()
            .map(|color| *color)
            .map_err(|_| RuntimeError::BorrowedTransient("Color.white"))
    })
}

#[derive(Debug)]
pub struct RuntimeFluidState {
    pub temperature: f32,
    color: FluidColorRef,
    properties: FluidPropertiesRef,
    pub translated_container_name: Option<String>,
    pub name_cache: Option<String>,
    percentages: Vec<f32>,
    // A moved DTO can hold nonempty fluids without the native cache objects.
    // Its constructor host must initialize these before owner publication.
    initialized: bool,
}

#[derive(Debug)]
#[must_use]
pub struct FluidRecalcOutcome {
    pub recalculated: bool,
    /// A real owner pin for the host's subsequent dirty/notification phase.
    /// No component/entity guard may remain held while that phase executes.
    pub owner: Option<EntityOwner>,
}

impl RuntimeFluidState {
    pub(crate) fn from_value(body: &FluidContainer, default_color: Rgba) -> Self {
        Self {
            temperature: 22.0,
            color: Rc::new(RefCell::new(default_color)),
            properties: Rc::new(RefCell::new(SealedFluidProperties::default())),
            translated_container_name: None,
            name_cache: None,
            percentages: Vec::new(),
            initialized: body.fluids.is_empty(),
        }
    }

    pub fn color_slot(&self) -> &FluidColorRef {
        &self.color
    }
    pub fn properties_slot(&self) -> &FluidPropertiesRef {
        &self.properties
    }
    pub fn percentages(&self) -> &[f32] {
        &self.percentages
    }
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }
    pub(crate) fn mark_uninitialized(&mut self) {
        self.initialized = false;
    }

    pub(crate) fn check_archive_copy(&self) -> Result<(), RuntimeError> {
        let _color = self
            .color
            .try_borrow()
            .map_err(|_| RuntimeError::BorrowedTransient("FluidContainer.color"))?;
        let _properties = self
            .properties
            .try_borrow()
            .map_err(|_| RuntimeError::BorrowedTransient("FluidContainer.propertiesCache"))?;
        Ok(())
    }

    pub(crate) fn archive_copy(&self) -> Result<Self, RuntimeError> {
        let color = *self
            .color
            .try_borrow()
            .map_err(|_| RuntimeError::BorrowedTransient("FluidContainer.color"))?;
        let properties = self
            .properties
            .try_borrow()
            .map_err(|_| RuntimeError::BorrowedTransient("FluidContainer.propertiesCache"))?
            .clone();
        Ok(Self {
            temperature: self.temperature,
            color: Rc::new(RefCell::new(color)),
            properties: Rc::new(RefCell::new(properties)),
            translated_container_name: self.translated_container_name.clone(),
            name_cache: self.name_cache.clone(),
            percentages: self.percentages.clone(),
            initialized: self.initialized,
        })
    }

    /// Only the callback-free data phase. The context must be a reviewed pure
    /// lookup implementation. Unknown properties are never fabricated as zero.
    /// Native completed prefixes remain if an actual lookup fails mid-loop.
    pub(crate) fn recalculate(
        &mut self,
        body: &mut FluidContainer,
        ctx: &dyn EntityContext,
        force: bool,
        owner: Option<&EntityOwner>,
    ) -> Result<FluidRecalcOutcome, RuntimeError> {
        if !self.initialized && !force {
            return Err(RuntimeError::UnsupportedRuntime(
                "FluidContainer caches must be initialized before publication",
            ));
        }
        if !body.cache_invalidated && !force {
            return Ok(FluidRecalcOutcome {
                recalculated: false,
                owner: None,
            });
        }
        body.amount_cache = 0.0;
        for instance in body.fluids.iter().rev() {
            body.amount_cache += instance.amount;
        }
        let mut properties = self
            .properties
            .try_borrow_mut()
            .map_err(|_| RuntimeError::BorrowedTransient("FluidContainer.propertiesCache"))?;
        *properties = SealedFluidProperties::default();
        let mut color = [0.0f32; 3];
        self.percentages.clear();
        if body.amount_cache > 0.0 {
            for instance in &body.fluids {
                let percentage = instance.amount / body.amount_cache;
                self.percentages.push(percentage);
                let instance_color = FluidContainer::instance_color(instance, ctx);
                for channel in 0..3 {
                    color[channel] += instance_color[channel] * percentage;
                }
                let Some(fluid) = &instance.fluid else {
                    return Err(RuntimeError::Codec(crate::EntityError(
                        "NullPointerException".into(),
                    )));
                };
                match ctx.fluid_properties(fluid) {
                    Lookup::Found(Some(values)) => {
                        add_properties(&mut properties, &values, body.amount_cache * percentage)
                    }
                    Lookup::Found(None) => {}
                    Lookup::Missing => {
                        return Err(RuntimeError::Codec(crate::EntityError(
                            "NullPointerException".into(),
                        )))
                    }
                    Lookup::Unknown => {
                        return Err(RuntimeError::UnsupportedRuntime(
                            "FluidContainer fluid properties registry is unavailable",
                        ))
                    }
                }
            }
        }
        drop(properties);
        let mut cached_color = self
            .color
            .try_borrow_mut()
            .map_err(|_| RuntimeError::BorrowedTransient("FluidContainer.color"))?;
        if body.amount_cache > 0.0 {
            // Color.set(r,g,b) also writes alpha=1 in the installed class.
            *cached_color = [color[0], color[1], color[2], 1.0];
        } else {
            *cached_color = copy_default_color()?;
        }
        drop(cached_color);
        if body.capacity - body.amount_cache < 0.001 {
            body.amount_cache = body.capacity;
        }
        self.name_cache = None;
        body.cache_invalidated = false;
        self.initialized = true;
        Ok(FluidRecalcOutcome {
            recalculated: true,
            owner: owner.cloned(),
        })
    }

    /// Native reset/Empty mutate the existing final cache objects. The
    /// receiver is detached before pool reset, so no owner callback runs.
    pub(crate) fn reset_detached(&mut self, body: &mut FluidContainer) -> Result<(), RuntimeError> {
        let mut properties = self
            .properties
            .try_borrow_mut()
            .map_err(|_| RuntimeError::BorrowedTransient("FluidContainer.propertiesCache"))?;
        let mut color = self
            .color
            .try_borrow_mut()
            .map_err(|_| RuntimeError::BorrowedTransient("FluidContainer.color"))?;
        body.fluids.clear();
        *properties = SealedFluidProperties::default();
        body.amount_cache = 0.0;
        *color = copy_default_color()?;
        body.capacity = 1.0;
        body.whitelist = None;
        body.blacklist = None;
        self.temperature = 22.0;
        body.container_name = "FluidContainer".into();
        self.translated_container_name = None;
        self.name_cache = None;
        body.cache_invalidated = false;
        body.input_locked = false;
        body.can_player_empty = true;
        body.hidden_amount = false;
        body.rain_catcher = 0.0;
        body.fills_with_clean_water = false;
        body.custom_drink_sound = None;
        self.percentages.clear();
        self.initialized = true;
        Ok(())
    }

    pub(crate) fn empty(
        &mut self,
        body: &mut FluidContainer,
        ctx: &dyn EntityContext,
        owner: Option<&EntityOwner>,
    ) -> Result<FluidRecalcOutcome, RuntimeError> {
        self.empty_without_recalculate(body)?;
        self.recalculate(body, ctx, true, owner)
    }

    pub(crate) fn empty_without_recalculate(
        &mut self,
        body: &mut FluidContainer,
    ) -> Result<(), RuntimeError> {
        let mut properties = self
            .properties
            .try_borrow_mut()
            .map_err(|_| RuntimeError::BorrowedTransient("FluidContainer.propertiesCache"))?;
        let mut color = self
            .color
            .try_borrow_mut()
            .map_err(|_| RuntimeError::BorrowedTransient("FluidContainer.color"))?;
        body.fluids.clear();
        *properties = SealedFluidProperties::default();
        body.amount_cache = 0.0;
        *color = copy_default_color()?;
        self.percentages.clear();
        self.initialized = true;
        Ok(())
    }
}

fn add_properties(target: &mut SealedFluidProperties, values: &SealedFluidProperties, amount: f32) {
    target.fatigue_change += values.fatigue_change * amount;
    target.hunger_change += values.hunger_change * amount;
    target.stress_change += values.stress_change * amount;
    target.thirst_change += values.thirst_change * amount;
    target.unhappy_change += values.unhappy_change * amount;
    target.calories += values.calories * amount;
    target.carbohydrates += values.carbohydrates * amount;
    target.lipids += values.lipids * amount;
    target.proteins += values.proteins * amount;
    target.alcohol += values.alcohol * amount;
    target.poison += values.poison * amount;
    target.flu_reduction += values.flu_reduction * amount;
    target.pain_reduction += values.pain_reduction * amount;
    target.endurance_change += values.endurance_change * amount;
    // Installed addFromMultiplied deliberately leaves foodSicknessChange alone.
}
