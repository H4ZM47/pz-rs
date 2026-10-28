//! How Project Zomboid draws vehicles in the iso world, on top of the
//! vehicle runtime (pz-vehicles):
//!
//! - [`instance`]: `VehicleModelInstance` — the skin's textures and the
//!   enable matrices `BaseVehicle.updateLights` / `doDamageOverlay` /
//!   `doBloodOverlay` fill for the vehicle shaders.
//! - [`models`]: `ModelManager.addVehicle` — the body and the part models
//!   (`BaseVehicle.ModelInfo`, `VehicleSubModelInstance`) with their
//!   animation players (`playPartAnim`, `updateAnimationPlayer`: doors,
//!   hoods and trunks posed open or closed).
//! - [`transform`]: `BaseVehicle.updateTransform` — wheels with suspension,
//!   steering and spin, attached part models.
//! - [`render`]: `ModelSlotRenderData` (vehicle branch) and
//!   `Model.DrawVehicle` as [`pz_render::model::VehicleDraw`]s, and
//!   `renderShadow` as an `FBORenderShadows` request; together the
//!   vehicle's place in `FBORenderCell.renderMovingObjects`.
//! - [`lights`]: headlights as lighting torches and the lightbar's
//!   lampposts.
//!
//! The vehicle shaders themselves are ported in pz-gpu (`vehicle.wgsl`).

pub mod instance;
pub mod joml;
pub mod lights;
pub mod models;
pub mod render;
pub mod transform;

pub use instance::{VehicleModelInstance, VehicleSkinTextures};
pub use models::{MediaVehicleModels, VehicleModel, VehicleModelSet, VehicleModelSource};
pub use render::{render_vehicle_object, vehicle_draws, vehicle_shadow, VehicleFrame};
