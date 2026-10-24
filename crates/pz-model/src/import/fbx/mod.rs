//! FBX, binary (7.x) and ASCII, through assimp 5.3's FBX importer:
//! tokenizer, element tree, DOM and converter.

mod convert;
mod document;
mod parser;

use super::scene::AiScene;
use super::ImportError;

/// `FBXImporter::InternReadFile` with the importer's default settings
/// (all geometry layers, materials, textures, weights and animations;
/// pivots preserved as `$AssimpFbx$` nodes).
pub fn import(data: &[u8]) -> Result<AiScene, ImportError> {
    let mut contents = data.to_vec();
    contents.push(0);
    let tokens = if contents.starts_with(b"Kaydara FBX Binary") {
        parser::tokenize_binary(&contents)?
    } else {
        parser::tokenize_ascii(&contents)?
    };
    let root = parser::parse(&tokens)?;
    let doc = document::Document::new(&root, document::ImportSettings::default())?;
    let scene = convert::Converter::convert(&doc)?;
    let usf = doc.globals.get_or::<f32>("UnitScaleFactor", 1.0)?;
    if usf == 0.0 {
        return Err(ImportError(
            "FBX: The UnitScaleFactor must be non-zero".into(),
        ));
    }
    Ok(scene)
}
