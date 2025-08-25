mod adminmodule;
mod authz;
mod bank;
mod dex;
mod gov;
mod slinky;
mod tokenfactory;
mod wasm;

#[macro_use]
pub mod macros;

pub trait Module<'a, R: Runner<'a>> {
    fn new(runner: &'a R) -> Self;
}

pub use adminmodule::Admin;
pub use authz::Authz;
pub use bank::Bank;
pub use dex::Dex;
pub use gov::Gov;
pub use gov::GovWithAppAccess;
pub use slinky::Slinky;
pub use tokenfactory::TokenFactory;
pub use wasm::Wasm;

use crate::Runner;
