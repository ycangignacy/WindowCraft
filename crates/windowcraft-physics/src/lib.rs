// Made by ycangignacy
//! Motion primitives for the WindowCraft interface.
//!
//! Everything here is pure arithmetic with no UI dependency, so the behaviour
//! of springs and gestures can be verified by unit tests.

pub mod animator;
pub mod gesture;
pub mod momentum;
pub mod spring;
pub mod velocity;

pub use animator::{Channel, MotionMode, Timeline};
pub use gesture::{Axis, DragGesture, DragPhase, PressTracker, Release};
pub use momentum::{project_momentum, rubberband};
pub use spring::{SETTLE_EPSILON, Spring1D, Spring2D, SpringConfig, relative_velocity};
pub use velocity::VelocityTracker;
