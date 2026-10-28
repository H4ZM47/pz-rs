//! `zombie.scripting.entity.components.attributes`.

mod attributes_script;

pub use attributes_script::{
    attribute_type_from_name, install_attribute_template_creator, take_retired_attribute_templates,
    AttributeTemplate, AttributeTemplateCreator, AttributesScript,
};
