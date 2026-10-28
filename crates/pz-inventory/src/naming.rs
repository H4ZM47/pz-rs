//! `InventoryItem.setName`, `nameAfterDescriptor` / `monogramAfterDescriptor`,
//! and explicitly named archive texture requests. Actual Native captured
//! texture fields/getters/producers live in crate::item_texture; these archive
//! name requests never reconstruct a live item asset.

use crate::item::InventoryItem;
use crate::moddata::ModDataResult;

/// `media/inventory/Question_On.png`: the texture of an icon that does not
/// load.
pub const QUESTION_TEXTURE: &str = "media/inventory/Question_On.png";

/// How `texture` was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemTexture {
    /// `Texture.trygetTexture(name)`, falling back to [`QUESTION_TEXTURE`]
    /// when `fallback`.
    Try { name: String, fallback: bool },
    /// `Texture.getSharedTexture(name)` (a texture name read from a save).
    Shared(String),
}

/// `String.substring(0, 256)` in UTF-16 units.
fn truncate_utf16(s: &str, max: usize) -> &str {
    let mut units = 0;
    for (i, c) in s.char_indices() {
        units += c.len_utf16();
        if units > max {
            return &s[..i];
        }
    }
    s
}

/// `StringUtils.isNullOrWhitespace`.
fn blank(s: &str) -> bool {
    s.chars().all(char::is_whitespace)
}

impl InventoryItem {
    /// `setName(name)`: at most 256 characters.
    pub fn set_name(&mut self, name: &str) {
        self.name = truncate_utf16(name, 256).to_string();
    }

    /// `nameAfterDescriptor(desc)`: `display` is
    /// `Translator.getText(scriptItem.getDisplayName())`; a `null` name is
    /// Java's "null".
    pub fn name_after_descriptor(
        &mut self,
        display: &str,
        forename: Option<&str>,
        surname: Option<&str>,
    ) {
        let name = format!(
            "{display}: {} {}",
            forename.unwrap_or("null"),
            surname.unwrap_or("null")
        );
        self.set_name(&name);
    }

    /// `monogramAfterDescriptor(desc)`: the initials; `Err` where
    /// `charAt(0)` throws (a `null` or empty name).
    pub fn monogram_after_descriptor(
        &mut self,
        display: &str,
        forename: Option<&str>,
        surname: Option<&str>,
    ) -> Result<(), &'static str> {
        let initial = |s: Option<&str>| -> Result<char, &'static str> {
            let s = s.ok_or("java.lang.NullPointerException")?;
            // `charAt(0)`: the first UTF-16 unit.
            let u = s
                .encode_utf16()
                .next()
                .ok_or("java.lang.StringIndexOutOfBoundsException")?;
            Ok(char::from_u32(u as u32).unwrap_or('\u{fffd}'))
        };
        let (f, s) = (initial(forename)?, initial(surname)?);
        self.set_name(&format!("{display}: {f}{s}"));
        Ok(())
    }

    /// The `customInventoryIcon` mod-data string `getTexture()` prefers
    /// (when that texture exists).
    pub fn try_custom_inventory_icon(&self) -> ModDataResult<Option<String>> {
        // Native getTexture checks own nonemptiness before rawget, then
        // instanceof String. Non-string values fall back to the texture field.
        if self
            .table
            .as_ref()
            .map(|table| table.try_is_empty())
            .transpose()?
            .unwrap_or(true)
        {
            return Ok(None);
        }
        Ok(match self.mod_data_value("customInventoryIcon")? {
            Some(pz_kahlua::Value::Str(value)) => Some(value.to_string()),
            _ => None,
        })
    }
    /// Legacy synchronous archive adapter; runtime texture lookup propagates
    /// `try_custom_inventory_icon` failures instead.
    pub fn custom_inventory_icon(&self) -> Option<String> {
        self.try_custom_inventory_icon()
            .expect("unborrowed custom inventory icon ModData")
    }

    /// The `texture` field: the script's `normalTexture` (its `Icon`, or
    /// the first `IconsForTexture` entry), a texture name from the save,
    /// then `synchWithVisual`'s choice (the `IconsForTexture` entry of the
    /// model index, then for clothing and bags the visual's base texture or
    /// texture choice). `clothing_base_textures` is the item's
    /// `ClothingItem.baseTextures.size()` when it has a visual and a
    /// clothing item (`None` otherwise). `None`: no texture.
    pub fn archive_texture_request(
        &self,
        clothing_base_textures: Option<usize>,
    ) -> pz_entity::runtime_load::HostResult<Option<ItemTexture>> {
        if self.textures.live_slot().is_some() {
            return Err(pz_entity::runtime_load::RuntimeHostError::Unsupported {
                operation: "name reconstruction is archive data projection only",
                kind: None,
            });
        }
        let icons = self.template.icons_for_texture.as_deref();
        let mut t = match &self.template.icon {
            Some(icon) => Some(ItemTexture::Try {
                name: format!("Item_{icon}"),
                fallback: true,
            }),
            None => icons.and_then(|i| i.first()).map(|i| ItemTexture::Try {
                name: format!("Item_{i}"),
                fallback: false,
            }),
        };
        if let crate::item_texture::ItemTextures::Archive(profile) = &self.textures {
            if let Some(name) = &profile.saved_name {
                t = Some(ItemTexture::Shared(name.clone()));
            }
        }
        if self.model_index != -1 {
            let icon = icons.and_then(|i| {
                usize::try_from(self.model_index)
                    .ok()
                    .and_then(|m| i.get(m))
            });
            if let Some(icon) = icon.filter(|s| !blank(s)) {
                t = Some(ItemTexture::Try {
                    name: format!("Item_{icon}"),
                    fallback: true,
                });
            }
        }
        if let (Some(base_textures), Some(v), Some(icons)) =
            (clothing_base_textures, &self.visual, icons)
        {
            if base_textures > 1 || v.texture_choice > -1 {
                let pick = |i: i8| usize::try_from(i).ok().and_then(|i| icons.get(i));
                let icon = if v.base_texture > -1 && icons.len() > v.base_texture as usize {
                    pick(v.base_texture)
                } else if v.texture_choice > -1 && icons.len() > v.texture_choice as usize {
                    pick(v.texture_choice)
                } else {
                    None
                };
                if let Some(icon) = icon.filter(|s| !blank(s)) {
                    t = Some(ItemTexture::Try {
                        name: format!("Item_{icon}"),
                        fallback: true,
                    });
                }
            }
        }
        Ok(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::{ItemClass, ItemTemplate};
    use std::rc::Rc;

    fn item(f: impl FnOnce(&mut ItemTemplate)) -> InventoryItem {
        let mut t = ItemTemplate::new(ItemClass::Normal, "Base", "IDcard");
        f(&mut t);
        InventoryItem::create(Rc::new(t), 1)
    }

    #[test]
    fn names() {
        let mut i = item(|_| {});
        i.name_after_descriptor("ID Card", Some("Kate"), None);
        assert_eq!(i.name, "ID Card: Kate null");
        i.monogram_after_descriptor("Badge", Some("Kate"), Some("Baldwin"))
            .unwrap();
        assert_eq!(i.name, "Badge: KB");
        assert!(i
            .monogram_after_descriptor("Badge", Some(""), Some("B"))
            .is_err());
        i.set_name(&"\u{1F600}".repeat(200));
        // 128 surrogate pairs fit in 256 UTF-16 units.
        assert_eq!(i.name.chars().count(), 128);
    }

    #[test]
    fn textures() {
        assert_eq!(item(|_| {}).archive_texture_request(None).unwrap(), None);
        let i = item(|t| t.icon = Some("IDcard".into()));
        assert_eq!(
            i.archive_texture_request(None).unwrap(),
            Some(ItemTexture::Try {
                name: "Item_IDcard".into(),
                fallback: true
            })
        );
        let mut i = item(|t| t.icons_for_texture = Some(vec!["A".into(), "B".into()]));
        assert_eq!(
            i.archive_texture_request(None).unwrap(),
            Some(ItemTexture::Try {
                name: "Item_A".into(),
                fallback: false
            })
        );
        i.model_index = 1;
        assert_eq!(
            i.archive_texture_request(None).unwrap(),
            Some(ItemTexture::Try {
                name: "Item_B".into(),
                fallback: true
            })
        );
        i.model_index = -1;
        i.textures = crate::item_texture::ItemTextures::Archive(
            crate::item_texture::ArchiveTextureProfile {
                saved_name: Some("Item_Saved".into()),
            },
        );
        assert_eq!(
            i.archive_texture_request(None).unwrap(),
            Some(ItemTexture::Shared("Item_Saved".into()))
        );
    }
}
