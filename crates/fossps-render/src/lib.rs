//! The FOSSPS renderer: Vulkan 1.3, drawing into one window.
//!
//! [`Renderer`] owns the connection to the graphics card and the images shown
//! in the window. For now it draws a single triangle; it is the base the rest
//! of the renderer is built on.
//!
//! Building this crate needs no Vulkan SDK. The Vulkan library is found when
//! the game starts, and the shaders are compiled by the build script.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod error;
mod gpu;
#[cfg(test)]
mod offscreen;
mod renderer;
mod swapchain;
mod triangle;

pub use error::{RenderError, Version};
pub use gpu::{Gpu, validation_errors};
pub use renderer::{RenderConfig, Renderer};
