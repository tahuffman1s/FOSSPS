//! The FOSSPS renderer: Vulkan 1.3, drawing into one window.
//!
//! [`Renderer`] owns the connection to the graphics card and the images shown
//! in the window. Give it a scene read by `fossps_assets` with
//! [`Renderer::set_scene`] and a [`Camera`] to see it from. The scene is drawn
//! in high dynamic range, smaller than the window if [`Upscaling`] says so,
//! made full size by AMD's FSR 3.1, and tone mapped for the display as the
//! last step.
//!
//! Building this crate needs no Vulkan SDK. The Vulkan library is found when
//! the game starts, and the shaders are compiled by the build script.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod buffer;
mod camera;
mod error;
mod font;
mod gpu;
mod headless;
mod mesh;
#[cfg(test)]
mod offscreen;
mod overlay;
mod renderer;
mod scene;
mod swapchain;
mod tonemap;
mod ui_pass;
mod upscale;
mod view;

pub use camera::Camera;
pub use error::{RenderError, Version};
pub use font::Font;
pub use gpu::{Gpu, validation_errors};
pub use headless::{FrameTime, Headless};
pub use renderer::{RenderConfig, Renderer};
pub use upscale::Upscaling;
