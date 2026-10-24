//! Runtime tables item visuals read: `ClothingItem` assets
//! (`media/clothing/clothingItems/*.xml`, found through
//! `media/fileGuidTable.xml`) and `BloodClothingType` with the
//! `BloodBodyPartType` indices (read from the installed game's jar).
//!
//! Nothing here is shipped: [`ClothingItems::load`] reads the game's media
//! folder and [`BloodClothingTypes::from_jar`] its `projectzomboid.jar`.

use std::collections::HashMap;
use std::path::Path;

/// `zombie.core.skinnedmodel.population.ClothingItem`, as
/// `ClothingItemAssetManager.onFileTaskFinished` fills it from the XML.
#[derive(Clone, Debug, PartialEq)]
pub struct ClothingItem {
    /// `mame`: the file name without folder and `.xml`
    /// (`OutfitManager.extractClothingItemName` of the table path).
    pub name: String,
    /// `guid` (the table's, `OutfitManager.getClothingItem` sets it).
    pub guid: String,
    pub male_model: Option<String>,
    pub female_model: Option<String>,
    pub alt_male_model: Option<String>,
    pub alt_female_model: Option<String>,
    pub is_static: bool,
    pub base_textures: Vec<String>,
    pub attach_bone: Option<String>,
    pub masks: Vec<i32>,
    pub masks_folder: Option<String>,
    pub underlay_masks_folder: Option<String>,
    pub texture_choices: Vec<String>,
    pub allow_random_hue: bool,
    pub allow_random_tint: bool,
    pub decal_group: Option<String>,
    pub shader: Option<String>,
    pub hat_category: Option<String>,
    pub spawn_with: Vec<String>,
}

const MASKS_FOLDER_DEFAULT: &str = "media/textures/Body/Masks";
const CLOTHING_ITEMS_FOLDER: &str = "media/clothing/clothingItems/";

fn blank(s: &Option<String>) -> bool {
    s.as_deref().is_none_or(|s| s.trim().is_empty())
}

impl ClothingItem {
    /// An item with `ClothingItemXML`'s defaults.
    pub fn new(name: &str) -> Self {
        ClothingItem {
            name: name.to_string(),
            guid: String::new(),
            male_model: None,
            female_model: None,
            alt_male_model: None,
            alt_female_model: None,
            is_static: false,
            base_textures: Vec::new(),
            attach_bone: None,
            masks: Vec::new(),
            masks_folder: Some(MASKS_FOLDER_DEFAULT.into()),
            underlay_masks_folder: Some(MASKS_FOLDER_DEFAULT.into()),
            texture_choices: Vec::new(),
            allow_random_hue: false,
            allow_random_tint: false,
            decal_group: None,
            shader: None,
            hat_category: None,
            spawn_with: Vec::new(),
        }
    }

    /// `ClothingItemAssetManager.onFileTaskFinished`: the JAXB fields of
    /// `ClothingItemXML` with backslashes turned into slashes in the paths.
    /// `None` when the file is not well-formed (the asset fails to load).
    pub fn parse(name: &str, guid: &str, xml: &str) -> Option<Self> {
        let root = xml::parse_root(xml)?;
        if root.tag != "clothingItem" {
            return None;
        }
        let mut c = ClothingItem::new(name);
        c.guid = guid.to_string();
        let fix = |s: &str| s.replace('\\', "/");
        let mut base = Vec::new();
        let mut choices = Vec::new();
        let mut spawn = Vec::new();
        let mut masks = Vec::new();
        for (tag, text) in &root.children {
            match tag.as_str() {
                "m_MaleModel" => c.male_model = Some(fix(text)),
                "m_FemaleModel" => c.female_model = Some(fix(text)),
                "m_AltMaleModel" => c.alt_male_model = Some(fix(text)),
                "m_AltFemaleModel" => c.alt_female_model = Some(fix(text)),
                "m_Static" => c.is_static = xml::parse_bool(text),
                "m_BaseTextures" => base.push(fix(text)),
                "m_AttachBone" => c.attach_bone = Some(text.clone()),
                "m_Masks" => masks.push(xml::parse_int(text)?),
                "m_MasksFolder" => c.masks_folder = Some(fix(text)),
                "m_UnderlayMasksFolder" => c.underlay_masks_folder = Some(fix(text)),
                "textureChoices" => choices.push(fix(text)),
                "m_AllowRandomHue" => c.allow_random_hue = xml::parse_bool(text),
                "m_AllowRandomTint" => c.allow_random_tint = xml::parse_bool(text),
                "m_DecalGroup" => c.decal_group = Some(text.clone()),
                "m_Shader" => c.shader = Some(text.clone()),
                "m_HatCategory" => c.hat_category = Some(text.clone()),
                "m_SpawnWith" => spawn.push(fix(text)),
                _ => {}
            }
        }
        c.base_textures = base;
        c.texture_choices = choices;
        c.spawn_with = spawn;
        c.masks = masks;
        Some(c)
    }

    /// `isHat()`.
    pub fn is_hat(&self) -> bool {
        !blank(&self.hat_category) && self.hat_category.as_deref() != Some("nobeard")
    }

    /// `isMask()`.
    pub fn is_mask(&self) -> bool {
        match self.hat_category.as_deref() {
            Some(h) if !h.trim().is_empty() => h == "nohairnobeard" || !h.contains("hair"),
            _ => false,
        }
    }

    /// `hasModel()`.
    pub fn has_model(&self) -> bool {
        !blank(&self.male_model) && !blank(&self.female_model)
    }
}

/// `FileGuidTable` (game files only) and the `ClothingItem` assets it
/// names, loaded the way `OutfitManager.getClothingItem(guid)` loads them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClothingItems {
    /// `pathToGuid` (case-insensitive: keys lowercased).
    path_to_guid: HashMap<String, String>,
    /// Loaded assets by guid; `None` for an asset that failed to load
    /// (`isReady()` false).
    by_guid: HashMap<String, Option<ClothingItem>>,
}

impl ClothingItems {
    /// Reads `media/fileGuidTable.xml` under `game_dir` and every
    /// clothing item file it lists. Mod tables are not merged.
    pub fn load(game_dir: &Path) -> std::io::Result<Self> {
        let table = std::fs::read_to_string(game_dir.join("media/fileGuidTable.xml"))?;
        let pairs = xml::parse_guid_table(&table).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "fileGuidTable.xml")
        })?;
        let mut items = ClothingItems::default();
        for (path, guid) in pairs {
            // FileGuidTable.setModID("game").
            let guid = format!("game-{guid}");
            if path.starts_with(CLOTHING_ITEMS_FOLDER) {
                let name = path
                    .trim_start_matches(CLOTHING_ITEMS_FOLDER)
                    .trim_end_matches(".xml")
                    .to_string();
                let asset = std::fs::read(game_dir.join(&path))
                    .ok()
                    .and_then(|b| String::from_utf8(b).ok())
                    .and_then(|x| ClothingItem::parse(&name, &guid, &x));
                items.by_guid.entry(guid.clone()).or_insert(asset);
            }
            items.path_to_guid.insert(path.to_lowercase(), guid);
        }
        Ok(items)
    }

    /// Adds an asset under a table path `media/clothing/clothingItems/<name>.xml`
    /// (for tests and tools; `None` adds a failed asset).
    pub fn insert(&mut self, name: &str, guid: &str, item: Option<ClothingItem>) {
        let path = format!("{CLOTHING_ITEMS_FOLDER}{name}.xml");
        self.path_to_guid
            .insert(path.to_lowercase(), guid.to_string());
        self.by_guid.insert(guid.to_string(), item);
    }

    /// `OutfitManager.loadAllClothingItems` for one script name: the ready
    /// asset of `media/clothing/clothingItems/<name>.xml`, if any.
    pub fn get(&self, name: &str) -> Option<&ClothingItem> {
        if name.trim().is_empty() {
            return None;
        }
        let path = format!("{CLOTHING_ITEMS_FOLDER}{name}.xml").to_lowercase();
        let guid = self.path_to_guid.get(&path)?;
        self.by_guid.get(guid)?.as_ref()
    }

    /// Number of ready assets.
    pub fn len(&self) -> usize {
        self.by_guid.values().filter(|v| v.is_some()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// `BloodClothingType` (covered parts, parents first) and the
/// `BloodBodyPartType` indices.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BloodClothingTypes {
    /// `BY_NAME`: name to `coveredParts` (as `BloodBodyPartType` indices).
    types: HashMap<String, Vec<usize>>,
    /// `BloodBodyPartType` names by index.
    pub body_parts: Vec<String>,
}

impl BloodClothingTypes {
    /// Builds the table from `(name, covered parts)` and the body part
    /// names in index order.
    pub fn new(types: Vec<(String, Vec<usize>)>, body_parts: Vec<String>) -> Self {
        BloodClothingTypes {
            types: types.into_iter().collect(),
            body_parts,
        }
    }

    /// Reads both enums' constructor arguments from the game's jar.
    pub fn from_jar(jar: &pz_classfile::Jar) -> pz_classfile::Result<Self> {
        use pz_classfile::Value;
        const PART: &str = "zombie/characterTextures/BloodBodyPartType";
        const CLOTHING: &str = "zombie/characterTextures/BloodClothingType";
        let mut parts: Vec<(i64, String)> = Vec::new();
        let cf = pz_classfile::load_class(jar, PART)?;
        for init in pz_classfile::static_inits(&cf)? {
            if let Value::New { class, args } = &init.value {
                if class == PART {
                    if let (Some(Value::Str(n)), Some(Value::Int(i))) = (args.first(), args.get(1))
                    {
                        parts.push((*i, n.clone()));
                    }
                }
            }
        }
        parts.sort();
        let names: Vec<String> = parts.into_iter().map(|(_, n)| n).collect();
        let index = |v: &Value| match v {
            Value::Static { class, name } if class == PART => names.iter().position(|n| n == name),
            _ => None,
        };
        let cf = pz_classfile::load_class(jar, CLOTHING)?;
        let mut types: HashMap<String, Vec<usize>> = HashMap::new();
        for init in pz_classfile::static_inits(&cf)? {
            let Value::New { class, args } = &init.value else {
                continue;
            };
            if class != CLOTHING {
                continue;
            }
            let Some(Value::Str(name)) = args.first() else {
                continue;
            };
            let mut covered = Vec::new();
            for a in &args[2..] {
                match a {
                    Value::Static { class, name } if class == CLOTHING => {
                        covered.extend(types.get(name).cloned().unwrap_or_default());
                    }
                    Value::Array(parts) => covered.extend(parts.iter().filter_map(index)),
                    _ => {}
                }
            }
            types.insert(name.clone(), covered);
        }
        Ok(BloodClothingTypes {
            types,
            body_parts: names,
        })
    }

    /// `BloodClothingType.fromString(name).coveredParts`.
    pub fn covered_parts(&self, name: &str) -> Option<&[usize]> {
        self.types.get(name).map(Vec::as_slice)
    }

    /// `BloodClothingType.getCoveredParts(list)`: unknown names add
    /// nothing (`Item.DoParam` drops them when parsing the script).
    pub fn covered_parts_of(&self, names: &[String]) -> Vec<usize> {
        names
            .iter()
            .filter_map(|n| self.covered_parts(n))
            .flatten()
            .copied()
            .collect()
    }

    /// `BloodBodyPartType.valueOf(name).index()`.
    pub fn part_index(&self, name: &str) -> Option<usize> {
        self.body_parts.iter().position(|n| n == name)
    }

    /// `BloodBodyPartType.MAX.index()`.
    pub fn max(&self) -> usize {
        self.part_index("MAX").unwrap_or(self.body_parts.len())
    }
}

/// What the item visuals read at runtime.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemAssets {
    pub clothing_items: ClothingItems,
    pub blood_clothing_types: BloodClothingTypes,
}

impl ItemAssets {
    /// Loads both tables from the game folder (`.../Contents/Java`).
    pub fn load(game_dir: &Path) -> Result<Self, String> {
        let clothing_items = ClothingItems::load(game_dir).map_err(|e| e.to_string())?;
        let jar = pz_classfile::Jar::open(game_dir.join("projectzomboid.jar"))
            .map_err(|e| e.to_string())?;
        let blood_clothing_types = BloodClothingTypes::from_jar(&jar).map_err(|e| e.to_string())?;
        Ok(ItemAssets {
            clothing_items,
            blood_clothing_types,
        })
    }
}

/// The XML subset these files use: one root element whose children are
/// text-only elements (JAXB reads the text verbatim).
mod xml {
    pub struct Root {
        pub tag: String,
        pub children: Vec<(String, String)>,
    }

    fn unescape(s: &str) -> Option<String> {
        if !s.contains('&') {
            return Some(s.to_string());
        }
        let mut out = String::new();
        let mut rest = s;
        while let Some(i) = rest.find('&') {
            out.push_str(&rest[..i]);
            let end = rest[i..].find(';')? + i;
            let ent = &rest[i + 1..end];
            match ent {
                "amp" => out.push('&'),
                "lt" => out.push('<'),
                "gt" => out.push('>'),
                "quot" => out.push('"'),
                "apos" => out.push('\''),
                _ => {
                    let n = if let Some(h) = ent.strip_prefix("#x") {
                        u32::from_str_radix(h, 16).ok()?
                    } else {
                        ent.strip_prefix('#')?.parse().ok()?
                    };
                    out.push(char::from_u32(n)?);
                }
            }
            rest = &rest[end + 1..];
        }
        out.push_str(rest);
        Some(out)
    }

    /// Skips the prolog, comments and whitespace before the next tag.
    fn skip_misc(s: &str) -> Option<&str> {
        let mut s = s.trim_start_matches('\u{feff}');
        loop {
            s = s.trim_start();
            if let Some(r) = s.strip_prefix("<?") {
                s = &r[r.find("?>")? + 2..];
            } else if let Some(r) = s.strip_prefix("<!--") {
                s = &r[r.find("-->")? + 3..];
            } else {
                return Some(s);
            }
        }
    }

    /// `<tag attrs>` or `<tag/>` at the start: (tag, self-closing, rest).
    fn open_tag(s: &str) -> Option<(&str, bool, &str)> {
        let r = s.strip_prefix('<')?;
        let end = r.find('>')?;
        let inner = &r[..end];
        let closed = inner.ends_with('/');
        let inner = inner.trim_end_matches('/');
        let tag = inner.split_whitespace().next()?;
        Some((tag, closed, &r[end + 1..]))
    }

    pub fn parse_root(s: &str) -> Option<Root> {
        let s = skip_misc(s)?;
        let (tag, closed, mut rest) = open_tag(s)?;
        let mut root = Root {
            tag: tag.to_string(),
            children: Vec::new(),
        };
        if closed {
            return Some(root);
        }
        loop {
            rest = skip_misc(rest)?;
            if let Some(r) = rest.strip_prefix("</") {
                let end = r.find('>')?;
                return (r[..end].trim() == root.tag).then_some(root);
            }
            let (child, closed, r) = open_tag(rest)?;
            if closed {
                root.children.push((child.to_string(), String::new()));
                rest = r;
                continue;
            }
            let close = format!("</{child}>");
            let end = r.find(&close)?;
            let raw = &r[..end];
            if raw.contains('<') {
                // Nested elements: not part of these formats.
                return None;
            }
            root.children.push((child.to_string(), unescape(raw)?));
            rest = &r[end + close.len()..];
        }
    }

    /// JAXB `xs:boolean` (`true`/`1`; anything else false).
    pub fn parse_bool(s: &str) -> bool {
        matches!(s.trim(), "true" | "1")
    }

    pub fn parse_int(s: &str) -> Option<i32> {
        s.trim().trim_start_matches('+').parse().ok()
    }

    /// `fileGuidTable`: `<files><path/><guid/></files>` pairs, in order.
    pub fn parse_guid_table(s: &str) -> Option<Vec<(String, String)>> {
        let mut out = Vec::new();
        let mut rest = s;
        while let Some(i) = rest.find("<files>") {
            let r = &rest[i + 7..];
            let end = r.find("</files>")?;
            let block = &r[..end];
            let field = |name: &str| -> Option<String> {
                let open = format!("<{name}>");
                let a = block.find(&open)? + open.len();
                let b = block[a..].find(&format!("</{name}>"))? + a;
                unescape(&block[a..b])
            };
            if let (Some(p), Some(g)) = (field("path"), field("guid")) {
                out.push((p, g));
            }
            rest = &r[end + 8..];
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clothing_item_xml() {
        let x = "\u{feff}<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<clothingItem>\n\t<m_MaleModel>media\\models\\A.X</m_MaleModel>\n\t<m_GUID>g</m_GUID>\n\t<m_Static>false</m_Static>\n\t<m_AllowRandomTint>true</m_AllowRandomTint>\n\t<m_AttachBone></m_AttachBone>\n\t<m_Masks>3</m_Masks>\n\t<m_Masks>4</m_Masks>\n\t<!-- c -->\n\t<textureChoices>a\\b</textureChoices>\n\t<textureChoices>c &amp; d</textureChoices>\n\t<m_DecalGroup/>\n</clothingItem>";
        let c = ClothingItem::parse("Shirt", "game-g", x).unwrap();
        assert_eq!(c.male_model.as_deref(), Some("media/models/A.X"));
        assert!(c.allow_random_tint && !c.allow_random_hue && !c.is_static);
        assert_eq!(c.attach_bone.as_deref(), Some(""));
        assert_eq!(c.masks, vec![3, 4]);
        assert_eq!(c.texture_choices, vec!["a/b", "c & d"]);
        assert_eq!(c.decal_group.as_deref(), Some(""));
        assert_eq!(c.masks_folder.as_deref(), Some(MASKS_FOLDER_DEFAULT));
        assert!(ClothingItem::parse("x", "g", "<clothingItem><a>").is_none());
        assert!(!c.has_model());
    }

    #[test]
    fn guid_lookup() {
        let t = "<fileGuidTable><files><path>media/clothing/clothingItems/Hat_A.xml</path><guid>1</guid></files></fileGuidTable>";
        assert_eq!(
            xml::parse_guid_table(t).unwrap(),
            vec![("media/clothing/clothingItems/Hat_A.xml".into(), "1".into())]
        );
        let mut c = ClothingItems::default();
        c.insert("Hat_A", "game-1", Some(ClothingItem::new("Hat_A")));
        c.insert("Hat_B", "game-2", None);
        assert_eq!(c.get("hat_a").map(|c| c.name.as_str()), Some("Hat_A"));
        assert!(c.get("Hat_B").is_none() && c.get("Hat_C").is_none() && c.get(" ").is_none());
    }

    #[test]
    fn blood_types() {
        let b = BloodClothingTypes::new(
            vec![("Hat".into(), vec![2]), ("Shoes".into(), vec![0, 1])],
            vec![
                "Foot_L".into(),
                "Foot_R".into(),
                "Head".into(),
                "MAX".into(),
            ],
        );
        assert_eq!(
            b.covered_parts_of(&["Shoes".into(), "Nope".into(), "Hat".into()]),
            vec![0, 1, 2]
        );
        assert_eq!(b.max(), 3);
        assert_eq!(b.part_index("Head"), Some(2));
    }
}
