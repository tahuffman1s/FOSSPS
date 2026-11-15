//! The FOSSPS renderer: Vulkan 1.3, drawing into one window.
//!
//! [`Renderer`] owns the connection to the graphics card and the images shown
//! in the window. For now the scene is a single triangle. It is drawn in high
//! dynamic range and tone mapped for the display as the last step.
//!
//! Building this crate needs no Vulkan SDK. The Vulkan library is found when
//! the game starts, and the shaders are compiled by the build script.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod error;
mod gpu;
#[cfg(test)]
mod offscreen;
mod renderer;
mod scene;
mod swapchain;
mod tonemap;
mod triangle;

pub use error::{RenderError, Version};
pub use gpu::{Gpu, validation_errors};
pub use renderer::{RenderConfig, Renderer};
