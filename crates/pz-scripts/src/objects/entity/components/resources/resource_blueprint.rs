//! `zombie.entity.components.resources.ResourceBlueprint`: the parsed form
//! of a `id@type@io@capacity[@StackAny | @filter@channel@flags]` line.

use super::{ResourceChannel, ResourceFlag, ResourceIO, ResourceType};
use crate::base::ScriptResult;
use crate::java;
use crate::objects::entity::{illegal_argument, parse_float, parse_int};

const STACK_ANY_ITEM_IDENTIFIER: &str = "StackAny";
const STR_NULL: &str = "null";

#[derive(Clone, Debug, PartialEq)]
pub struct ResourceBlueprint {
    id: Option<String>,
    type_: ResourceType,
    io: ResourceIO,
    capacity: f32,
    channel: ResourceChannel,
    /// `EnumBitStore<ResourceFlag>` bits.
    flag_bits: i32,
    filter: Option<String>,
    stack_any_item: bool,
}

impl Default for ResourceBlueprint {
    /// The state after `reset()`.
    fn default() -> Self {
        ResourceBlueprint {
            id: None,
            type_: ResourceType::Any,
            io: ResourceIO::Any,
            capacity: 1.0,
            channel: ResourceChannel::NO_CHANNEL,
            flag_bits: 0,
            filter: None,
            stack_any_item: false,
        }
    }
}

impl ResourceBlueprint {
    /// `getId`.
    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    /// `getType`.
    pub fn resource_type(&self) -> ResourceType {
        self.type_
    }

    /// `getIO`.
    pub fn io(&self) -> ResourceIO {
        self.io
    }

    /// `getCapacity`.
    pub fn capacity(&self) -> f32 {
        self.capacity
    }

    /// `isStackAnyItem`.
    pub fn is_stack_any_item(&self) -> bool {
        self.stack_any_item
    }

    /// `getChannel`.
    pub fn channel(&self) -> ResourceChannel {
        self.channel
    }

    /// `hasFlag`.
    pub fn has_flag(&self, flag: ResourceFlag) -> bool {
        self.flag_bits & flag.bits() != 0
    }

    /// `getFlagBits`.
    pub fn flag_bits(&self) -> i32 {
        self.flag_bits
    }

    /// `getFilter`.
    pub fn filter(&self) -> Option<&str> {
        self.filter.as_deref()
    }

    /// `DeserializeFromScript`: flags given by name, `:`-separated.
    pub fn deserialize_from_script(serial: &str) -> ScriptResult<ResourceBlueprint> {
        Self::deserialize(serial, true)
    }

    /// `Deserialize(bp, serial, flagsAsString)`.
    pub fn deserialize(serial: &str, flags_as_string: bool) -> ScriptResult<ResourceBlueprint> {
        let elements = java::split_char(serial, '@');
        if !matches!(elements.len(), 4 | 5 | 7) {
            return Err(illegal_argument("Serial string has invalid number of elements."));
        }
        let mut bp = ResourceBlueprint { id: Some(elements[0].to_string()), ..Default::default() };
        bp.type_ = ResourceType::value_of(elements[1])?;
        bp.io = ResourceIO::value_of(elements[2])?;
        bp.capacity =
            if bp.type_ == ResourceType::Item { parse_int(elements[3])? as f32 } else { parse_float(elements[3])? };
        if elements.len() == 5 && java::eq_ignore_case(elements[4], STACK_ANY_ITEM_IDENTIFIER) {
            bp.stack_any_item = true;
        }
        if elements.len() == 7 {
            if !java::eq_ignore_case(STR_NULL, elements[4]) {
                bp.filter = Some(elements[4].to_string());
            }
            if !java::eq_ignore_case(STR_NULL, elements[5]) {
                bp.channel = ResourceChannel::value_of(elements[5])?;
            }
            let flags = elements[6];
            if !java::eq_ignore_case(STR_NULL, flags) {
                if flags_as_string {
                    for f in java::split_char(flags, ':') {
                        bp.flag_bits |= ResourceFlag::value_of(f)?.bits();
                    }
                } else {
                    bp.flag_bits = parse_int(flags)?;
                }
            }
        }
        Ok(bp)
    }
}
