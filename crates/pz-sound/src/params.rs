//! `zombie.audio.FMODParameter` and its local/global kinds, plus
//! `FMODParameterList`.
//!
//! A parameter recomputes its value each update from a [`ParameterSource`]
//! (the original's `calculateCurrentValue`) and pushes it to FMOD only when
//! it changed: local parameters to every event instance that uses them,
//! global ones to the studio system.

use crate::backend::{AudioBackend, InstanceHandle, ParameterDescription};

/// `calculateCurrentValue()` / `resetToDefault()` of a concrete parameter.
pub trait ParameterSource {
    fn calculate(&mut self) -> f32;
    fn reset_to_default(&mut self) {}
}

impl<F: FnMut() -> f32> ParameterSource for F {
    fn calculate(&mut self) -> f32 {
        self()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Scope {
    /// `FMODLocalParameter`: the event instances using it.
    Local(Vec<InstanceHandle>),
    /// `FMODGlobalParameter`.
    Global,
}

pub struct FmodParameter {
    name: String,
    description: Option<ParameterDescription>,
    current: f32,
    pub scope: Scope,
    source: Box<dyn ParameterSource>,
}

impl FmodParameter {
    /// `new FMODLocalParameter(name)`; the description comes from
    /// `FMODManager.getParameterDescription(name)`.
    pub fn local(
        name: &str,
        description: Option<ParameterDescription>,
        source: Box<dyn ParameterSource>,
    ) -> Self {
        FmodParameter {
            name: name.to_string(),
            description,
            current: f32::NAN,
            scope: Scope::Local(Vec::new()),
            source,
        }
    }

    /// `new FMODGlobalParameter(name)`.
    pub fn global(
        name: &str,
        description: Option<ParameterDescription>,
        source: Box<dyn ParameterSource>,
    ) -> Self {
        FmodParameter {
            name: name.to_string(),
            description,
            current: f32::NAN,
            scope: Scope::Global,
            source,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> Option<&ParameterDescription> {
        self.description.as_ref()
    }

    pub fn current_value(&self) -> f32 {
        self.current
    }

    fn id(&self) -> Option<(u32, u32)> {
        self.description.as_ref().map(|d| d.id)
    }

    /// `update()`: recompute; push only on change (`NaN` never equals, so
    /// the first update always pushes).
    pub fn update(&mut self, backend: &mut dyn AudioBackend) {
        let v = self.source.calculate();
        if v == self.current {
            return;
        }
        self.current = v;
        self.set_current_value(v, backend);
    }

    fn set_current_value(&self, v: f32, backend: &mut dyn AudioBackend) {
        let Some(id) = self.id() else { return };
        match &self.scope {
            Scope::Local(instances) => {
                for &inst in instances {
                    backend.instance_set_parameter_by_id(inst, id, v);
                }
            }
            Scope::Global => backend.system_set_parameter_by_id(id, v),
        }
    }

    pub fn reset_to_default(&mut self) {
        self.source.reset_to_default();
    }

    /// `startEventInstance(inst)`.
    pub fn start_event_instance(&mut self, inst: InstanceHandle, backend: &mut dyn AudioBackend) {
        let id = self.id();
        let v = self.current;
        if let Scope::Local(instances) = &mut self.scope {
            instances.push(inst);
            if let Some(id) = id {
                backend.instance_set_parameter_by_id(inst, id, v);
            }
        }
    }

    /// `stopEventInstance(inst)`: TLongArrayList.remove removes the first match.
    pub fn stop_event_instance(&mut self, inst: InstanceHandle) {
        if let Scope::Local(instances) = &mut self.scope {
            if let Some(i) = instances.iter().position(|&x| x == inst) {
                instances.remove(i);
            }
        }
    }
}

/// `FMODParameterList`.
#[derive(Default)]
pub struct FmodParameterList {
    pub parameters: Vec<FmodParameter>,
}

impl FmodParameterList {
    pub fn add(&mut self, p: FmodParameter) {
        self.parameters.push(p);
    }

    /// `get(description)`: by global index (the last added wins, as the
    /// array slot is overwritten).
    pub fn get(&self, d: &ParameterDescription) -> Option<&FmodParameter> {
        self.parameters.iter().rev().find(|p| {
            p.description()
                .is_some_and(|x| x.global_index == d.global_index)
        })
    }

    pub fn update(&mut self, backend: &mut dyn AudioBackend) {
        for p in &mut self.parameters {
            p.update(backend);
        }
    }
}
