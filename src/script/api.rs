//! openOMSI's plugin API as the game describes it (`assets/scripting/plugin-api.json`, written
//! by the game from its own registry): every function and event with its arguments and help.
//! The script editor completes, explains and checks names with it, and the test run makes a
//! stand-in of every function from it.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Param {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Function {
    /// Under `omsi.`: `map.ground`, `every`.
    pub name: String,
    #[serde(default)]
    pub since: String,
    #[serde(default)]
    pub doc: String,
    #[serde(default)]
    pub params: Vec<Param>,
    #[serde(default)]
    pub returns: String,
    /// The permission an `.oop` must declare to call it.
    #[serde(default)]
    pub permission: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Event {
    pub name: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub doc: String,
    #[serde(default)]
    pub since: String,
}

#[derive(Debug, Deserialize)]
pub struct Api {
    /// The game version the description is of.
    pub version: String,
    pub functions: Vec<Function>,
    pub events: Vec<Event>,
    #[serde(skip)]
    by_name: HashMap<String, usize>,
}

impl Api {
    pub fn function(&self, name: &str) -> Option<&Function> {
        self.by_name.get(name).map(|&i| &self.functions[i])
    }

    pub fn event(&self, name: &str) -> Option<&Event> {
        self.events.iter().find(|e| e.name == name)
    }

    /// The tables under `omsi.` (`map`, `fmt`, `object`...).
    pub fn groups(&self) -> Vec<&str> {
        let mut g: Vec<&str> = self.functions.iter().filter_map(|f| f.name.split_once('.').map(|(a, _)| a)).collect();
        g.sort_unstable();
        g.dedup();
        g
    }
}

impl Function {
    /// `omsi.map.ground(x, y)`: how it is called.
    pub fn signature(&self) -> String {
        let args: Vec<String> = self.params.iter().map(|p| if p.optional { format!("{}?", p.name) } else { p.name.clone() }).collect();
        format!("omsi.{}({})", self.name, args.join(", "))
    }

    /// A reading gives nil without the game; a change gives false (as the game does without
    /// the thing it changes).
    pub fn is_change(&self) -> bool {
        let r = self.returns.trim_start();
        r.starts_with("true") || r.starts_with("boolean")
    }
}

impl Event {
    pub fn signature(&self) -> String {
        format!("\"{}\" ({})", self.name, self.args.join(", "))
    }
}

/// The API, read once.
pub fn api() -> &'static Api {
    static API: OnceLock<Api> = OnceLock::new();
    API.get_or_init(|| {
        let mut api: Api = serde_json::from_str(include_str!("../../assets/scripting/plugin-api.json")).expect("plugin-api.json");
        api.by_name = api.functions.iter().enumerate().map(|(i, f)| (f.name.clone(), i)).collect();
        api
    })
}

/// Lua 5.4's own names a script can use (the libraries openOMSI keeps).
pub const LUA_GLOBALS: &[&str] = &[
    "assert", "error", "getmetatable", "ipairs", "load", "next", "pairs", "pcall", "print", "rawequal", "rawget", "rawlen", "rawset", "require", "select", "setmetatable", "tonumber", "tostring", "type", "xpcall", "_G", "_VERSION", "math", "string", "table", "utf8", "coroutine", "os", "package", "omsi",
];

/// The functions of the libraries, for completion after `math.` and the like.
pub const LUA_LIBRARY: &[&str] = &[
    "math.abs", "math.ceil", "math.cos", "math.deg", "math.exp", "math.floor", "math.fmod", "math.huge", "math.log", "math.max", "math.maxinteger", "math.min", "math.mininteger", "math.modf", "math.pi", "math.rad", "math.random", "math.randomseed", "math.sin", "math.sqrt", "math.tan", "math.tointeger", "math.type", "math.ult", "math.atan", "math.asin", "math.acos",
    "string.byte", "string.char", "string.find", "string.format", "string.gmatch", "string.gsub", "string.len", "string.lower", "string.match", "string.rep", "string.reverse", "string.sub", "string.upper", "string.pack", "string.unpack", "string.packsize",
    "table.concat", "table.insert", "table.move", "table.pack", "table.remove", "table.sort", "table.unpack",
    "utf8.char", "utf8.charpattern", "utf8.codepoint", "utf8.codes", "utf8.len", "utf8.offset",
    "coroutine.create", "coroutine.isyieldable", "coroutine.resume", "coroutine.running", "coroutine.status", "coroutine.wrap", "coroutine.yield", "coroutine.close",
    "os.clock", "os.date", "os.difftime", "os.time",
];

pub const LUA_KEYWORDS: &[&str] = &["and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in", "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_games_api_is_read() {
        let a = api();
        assert!(a.functions.len() > 250 && a.events.len() > 50);
        assert_eq!(a.function("map.ground").unwrap().signature(), "omsi.map.ground(x, y)");
        assert!(a.function("weather.set").unwrap().is_change());
        assert!(!a.function("map.name").unwrap().is_change());
        assert!(a.event("stop_arrive").is_some());
        assert!(a.groups().contains(&"fmt"));
    }
}
