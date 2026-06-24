pub mod signals;
mod navigate;
mod captcha;
mod consent;
mod input;
pub mod auth;
pub mod popup;

pub use navigate::{validate_url, validate_file_path, navigate};
pub use captcha::{is_captcha_present, resolve_captcha_loop};
pub use consent::handle_consent;
pub use input::{extract, type_and_submit};
