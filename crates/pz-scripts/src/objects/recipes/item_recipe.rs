//! `zombie.scripting.objects.ItemRecipe`: one item's entry in an evolved
//! recipe's ingredient list (built from an item's `EvolvedRecipe` param).

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemRecipe {
    pub name: String,
    pub use_: i32,
    pub cooked: bool,
    module: String,
}

impl ItemRecipe {
    pub fn new(name: &str, module: &str, use_: i32) -> Self {
        ItemRecipe { name: name.to_string(), use_, cooked: false, module: module.to_string() }
    }

    /// `getUse`.
    pub fn use_(&self) -> i32 {
        self.use_
    }

    /// `getName`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// `getModule`.
    pub fn module(&self) -> &str {
        &self.module
    }

    /// `setModule`.
    pub fn set_module(&mut self, module: &str) {
        self.module = module.to_string();
    }

    /// `getFullType`.
    pub fn full_type(&self) -> String {
        format!("{}.{}", self.module, self.name)
    }
}
