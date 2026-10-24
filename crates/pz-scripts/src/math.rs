//! Small vector types standing in for `org.joml.Vector2f`/`Vector3f`/
//! `Vector4f`/`Quaternionf` and `zombie.iso.Vector2`/`Vector3` in script
//! data. Engine crates may convert these to their own math types.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vector2 {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vector3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vector4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Vector2 {
    pub fn new(x: f32, y: f32) -> Self {
        Vector2 { x, y }
    }
    pub fn set(&mut self, x: f32, y: f32) {
        *self = Vector2 { x, y };
    }
    pub fn mul(&mut self, s: f32) {
        self.x *= s;
        self.y *= s;
    }
}

impl Vector3 {
    pub fn new(x: f32, y: f32, z: f32) -> Self {
        Vector3 { x, y, z }
    }
    pub fn set(&mut self, x: f32, y: f32, z: f32) {
        *self = Vector3 { x, y, z };
    }
    pub fn mul(&mut self, s: f32) {
        self.x *= s;
        self.y *= s;
        self.z *= s;
    }
}

impl Vector4 {
    pub fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Vector4 { x, y, z, w }
    }
}
