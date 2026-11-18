//! A map's scripts: the project of Lua scripts and modules openOMSI runs while the map is
//! played (`project`), the game's API they call (`api`), what the editor colours and checks
//! (`lex`, `lint`), a test run with a debugger (`run`) and building them into an `.oop`
//! (`build`).

pub mod api;
pub mod build;
pub mod lex;
pub mod lint;
pub mod project;
pub mod run;
