//! `CharacterMask` (which body regions of the skin/underlayer stay visible
//! under clothing) and `BloodBodyPartType`.

/// `CharacterMask.Part`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    Head,
    Torso,
    Pelvis,
    LeftArm,
    LeftHand,
    RightArm,
    RightHand,
    LeftLeg,
    LeftFoot,
    RightLeg,
    RightFoot,
    Dress,
    Chest,
    Waist,
    Belt,
    Crotch,
}

impl Part {
    pub const ALL: [Part; 16] = [
        Part::Head,
        Part::Torso,
        Part::Pelvis,
        Part::LeftArm,
        Part::LeftHand,
        Part::RightArm,
        Part::RightHand,
        Part::LeftLeg,
        Part::LeftFoot,
        Part::RightLeg,
        Part::RightFoot,
        Part::Dress,
        Part::Chest,
        Part::Waist,
        Part::Belt,
        Part::Crotch,
    ];

    /// `Part.leaves()`: the parts without subdivisions, in enum order.
    pub fn leaves() -> impl Iterator<Item = Part> {
        Part::ALL.into_iter().filter(|p| !p.has_subdivisions())
    }

    pub fn from_int(i: i32) -> Option<Part> {
        Part::ALL.get(usize::try_from(i).ok()?).copied()
    }

    pub fn index(self) -> usize {
        self as usize
    }

    /// The `toString` used in mask texture names (`<folder>/<Part>.png`).
    pub fn name(self) -> &'static str {
        match self {
            Part::Head => "Head",
            Part::Torso => "Torso",
            Part::Pelvis => "Pelvis",
            Part::LeftArm => "LeftArm",
            Part::LeftHand => "LeftHand",
            Part::RightArm => "RightArm",
            Part::RightHand => "RightHand",
            Part::LeftLeg => "LeftLeg",
            Part::LeftFoot => "LeftFoot",
            Part::RightLeg => "RightLeg",
            Part::RightFoot => "RightFoot",
            Part::Dress => "Dress",
            Part::Chest => "Chest",
            Part::Waist => "Waist",
            Part::Belt => "Belt",
            Part::Crotch => "Crotch",
        }
    }

    pub fn has_subdivisions(self) -> bool {
        matches!(self, Part::Torso | Part::Pelvis)
    }

    pub fn sub_divisions(self) -> &'static [Part] {
        match self {
            Part::Torso => &[Part::Chest, Part::Waist],
            Part::Pelvis => &[Part::Belt, Part::Crotch],
            _ => &[],
        }
    }

    /// `getBloodBodyPartTypes`.
    pub fn blood_body_part_types(self) -> &'static [BloodBodyPartType] {
        use BloodBodyPartType as B;
        match self {
            Part::Head => &[B::Head],
            Part::Torso => &[B::TorsoUpper, B::TorsoLower],
            Part::Pelvis => &[B::UpperLegL, B::UpperLegR, B::Groin],
            Part::LeftArm => &[B::UpperArmL, B::ForeArmL],
            Part::LeftHand => &[B::HandL],
            Part::RightArm => &[B::UpperArmR, B::ForeArmR],
            Part::RightHand => &[B::HandR],
            Part::LeftLeg => &[B::UpperLegL, B::LowerLegL],
            Part::LeftFoot => &[B::FootL],
            Part::RightLeg => &[B::UpperLegR, B::LowerLegR],
            Part::RightFoot => &[B::FootR],
            Part::Dress => &[],
            Part::Chest => &[B::TorsoUpper],
            Part::Waist => &[B::TorsoLower],
            Part::Belt => &[B::UpperLegL, B::UpperLegR],
            Part::Crotch => &[B::Groin],
        }
    }
}

/// `CharacterMask`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharacterMask {
    visible: [bool; 16],
}

impl Default for CharacterMask {
    fn default() -> Self {
        CharacterMask {
            visible: [true; 16],
        }
    }
}

impl CharacterMask {
    pub fn set_all_visible(&mut self, v: bool) {
        self.visible = [v; 16];
    }

    pub fn set_part_visible(&mut self, part: Part, v: bool) {
        if part.has_subdivisions() {
            for &s in part.sub_divisions() {
                self.set_part_visible(s, v);
            }
        } else {
            self.visible[part.index()] = v;
        }
    }

    /// `setPartsVisible(ArrayList<Integer>, v)`: out-of-range values are
    /// skipped.
    pub fn set_parts_visible(&mut self, parts: &[i32], v: bool) {
        for &p in parts {
            if let Some(part) = Part::from_int(p) {
                self.set_part_visible(part, v);
            }
        }
    }

    pub fn is_part_visible(&self, part: Part) -> bool {
        if part.has_subdivisions() {
            return part.sub_divisions().iter().all(|s| self.visible[s.index()]);
        }
        self.visible[part.index()]
    }

    pub fn is_blood_body_part_visible(&self, b: BloodBodyPartType) -> bool {
        b.character_mask_parts()
            .iter()
            .any(|&p| self.is_part_visible(p))
    }

    fn is_all(&self, v: bool) -> bool {
        Part::leaves().all(|p| self.is_part_visible(p) == v)
    }

    pub fn is_nothing_visible(&self) -> bool {
        self.is_all(false)
    }

    pub fn is_all_visible(&self) -> bool {
        self.is_all(true)
    }

    /// `forEachVisible`: the visible leaves in enum order.
    pub fn visible_leaves(&self) -> Vec<Part> {
        Part::leaves()
            .filter(|&p| self.is_part_visible(p))
            .collect()
    }
}

/// `BloodBodyPartType` (without `MAX`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BloodBodyPartType {
    HandL,
    HandR,
    ForeArmL,
    ForeArmR,
    UpperArmL,
    UpperArmR,
    TorsoUpper,
    TorsoLower,
    Head,
    Neck,
    Groin,
    UpperLegL,
    UpperLegR,
    LowerLegL,
    LowerLegR,
    FootL,
    FootR,
    Back,
}

/// `BloodBodyPartType.MAX.index()`.
pub const BLOOD_PARTS: usize = 18;

impl BloodBodyPartType {
    pub const ALL: [BloodBodyPartType; BLOOD_PARTS] = {
        use BloodBodyPartType::*;
        [
            HandL, HandR, ForeArmL, ForeArmR, UpperArmL, UpperArmR, TorsoUpper, TorsoLower, Head,
            Neck, Groin, UpperLegL, UpperLegR, LowerLegL, LowerLegR, FootL, FootR, Back,
        ]
    };

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(i: usize) -> Option<Self> {
        Self::ALL.get(i).copied()
    }

    pub fn character_mask_parts(self) -> &'static [Part] {
        use BloodBodyPartType as B;
        match self {
            B::HandL => &[Part::LeftHand],
            B::HandR => &[Part::RightHand],
            B::ForeArmL | B::UpperArmL => &[Part::LeftArm],
            B::ForeArmR | B::UpperArmR => &[Part::RightArm],
            B::TorsoUpper => &[Part::Chest],
            B::TorsoLower => &[Part::Waist],
            B::Head | B::Neck => &[Part::Head],
            B::Groin => &[Part::Crotch],
            B::UpperLegL => &[Part::LeftLeg, Part::Pelvis],
            B::UpperLegR => &[Part::RightLeg, Part::Pelvis],
            B::LowerLegL => &[Part::LeftLeg],
            B::LowerLegR => &[Part::RightLeg],
            B::FootL => &[Part::LeftFoot],
            B::FootR => &[Part::RightFoot],
            B::Back => &[Part::Torso],
        }
    }
}

/// `CharacterSmartTexture.MaskFiles`.
pub const MASK_FILES: [&str; BLOOD_PARTS] = [
    "BloodMaskHandL",
    "BloodMaskHandR",
    "BloodMaskLArmL",
    "BloodMaskLArmR",
    "BloodMaskUArmL",
    "BloodMaskUArmR",
    "BloodMaskChest",
    "BloodMaskStomach",
    "BloodMaskHead",
    "BloodMaskNeck",
    "BloodMaskGroin",
    "BloodMaskULegL",
    "BloodMaskULegR",
    "BloodMaskLLegL",
    "BloodMaskLLegR",
    "BloodMaskFootL",
    "BloodMaskFootR",
    "BloodMaskBack",
];

const PATCH_PARTS: [&str; BLOOD_PARTS] = [
    "left_hand",
    "right_hand",
    "left_lower_arm",
    "right_lower_arm",
    "left_upper_arm",
    "right_upper_arm",
    "chest",
    "abdomen",
    "",
    "",
    "groin",
    "left_upper_leg",
    "right_upper_leg",
    "left_lower_leg",
    "right_lower_leg",
    "",
    "",
    "back",
];

/// `BasicPatchesMaskFiles` / `DenimPatchesMaskFiles` /
/// `LeatherPatchesMaskFiles` (`suffix` is `sheet`, `denim` or `leather`);
/// `None` where the original has an empty name.
pub fn patch_mask_file(part: BloodBodyPartType, suffix: &str) -> Option<String> {
    let p = PATCH_PARTS[part.index()];
    (!p.is_empty()).then(|| format!("patches_{p}_{suffix}"))
}
