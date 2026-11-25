//! A test run of a map's scripts in the editor, with a debugger: Lua 5.4 on a thread of its
//! own, sandboxed as openOMSI sandboxes a plugin, with the game's `omsi` table. What needs no
//! game works as in the game (events, timers, the clock, storage, the helpers, the map's name,
//! ground and objects); what needs a bus, a duty or the traffic answers as the game does
//! without them (`nil`, or `false` and the reason) and says so once in the console. The
//! clock runs at a chosen speed, the game's events can be sent by hand, and the run stops at
//! breakpoints: the stack, every frame's locals and upvalues, expressions evaluated in a
//! frame, steps over, into and out.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mlua::{Function, HookTriggers, Lua, LuaOptions, LuaSerdeExt, MultiValue, StdLib, Table, Value, VmState};

use crate::script::api::api;

/// A value of the script, as the debugger shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Val {
    pub text: String,
    pub ty: &'static str,
    /// A table that can be opened (`Cmd::Expand`).
    pub table: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct Frame {
    pub func: String,
    /// The project file (`scripts/x.lua`), or the chunk's name.
    pub file: String,
    pub line: usize,
    pub locals: Vec<(String, Val)>,
    pub upvalues: Vec<(String, Val)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// `print` and `omsi.log`.
    Print,
    /// What the game would show on the screen (`omsi.message`, a toast).
    Screen,
    Warn,
    Error,
    /// The run's own notes.
    Note,
    /// A console answer.
    Result,
}

#[derive(Debug, Clone)]
pub enum Out {
    Line { kind: LineKind, text: String, at: Option<(String, usize)> },
    /// Stopped in a script: why, where, the stack (innermost first).
    Paused { reason: String, file: String, line: usize, stack: Vec<Frame> },
    Resumed,
    /// The game clock (seconds since midnight), seconds since the start, whether time runs.
    Clock { clock: f64, time: f64, running: bool },
    /// What listens: handlers per event, timers.
    Listening { events: Vec<(String, usize)>, timers: usize },
    Expanded { id: usize, items: Vec<(String, Val)> },
    /// The plugin's storage and the objects' variables, as they are now.
    State { storage: Vec<(String, String)>, objects: Vec<(i64, Vec<(String, String)>)> },
    Ended(Option<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    Nil,
    Bool(bool),
    Num(f64),
    Str(String),
}

impl Arg {
    /// What is typed in an event's field: a number, true/false, nil, or text.
    pub fn parse(s: &str) -> Arg {
        let t = s.trim();
        if t.is_empty() || t == "nil" {
            Arg::Nil
        } else if t == "true" || t == "false" {
            Arg::Bool(t == "true")
        } else if let Ok(n) = t.parse::<f64>() {
            Arg::Num(n)
        } else {
            Arg::Str(t.trim_matches('"').to_string())
        }
    }
}

#[derive(Debug, Clone)]
pub enum Cmd {
    /// Time runs (or the script goes on from a breakpoint).
    Continue,
    /// Time stands still; a script running stops at its next line.
    Pause,
    StepOver,
    StepIn,
    StepOut,
    Stop,
    Breakpoints(HashMap<String, BTreeSet<usize>>),
    Fire(String, Vec<Arg>),
    /// Evaluated in a frame of the stack while stopped (its index), else globally.
    Eval(String, Option<usize>),
    Expand(usize),
    Speed(f64),
    SetClock(f64),
}

/// An object a script of objects drives.
#[derive(Debug, Clone)]
pub struct Obj {
    pub id: i64,
    pub file: String,
    pub pos: [f64; 3],
    pub heading: f64,
}

/// What the run starts from.
pub struct Setup {
    /// Lua files by their project path (`scripts/x.lua`).
    pub files: Vec<(String, String)>,
    /// The file that starts (`main.lua`, or an object script).
    pub entry: String,
    pub map_folder: String,
    pub friendly: String,
    pub map_path: String,
    pub lht: bool,
    /// The ground's height at a map point.
    pub ground: Arc<dyn Fn(f64, f64) -> Option<f64> + Send + Sync>,
    /// For a script of objects: the objects of its type on the map.
    pub objects: Vec<Obj>,
    /// Every object on the map (for `omsi.map.objects_near`): id, file, position.
    pub all_objects: Arc<Vec<(i64, String, [f64; 3])>>,
    pub clock: f64,
    pub speed: f64,
    pub breakpoints: HashMap<String, BTreeSet<usize>>,
    /// Stop at the first line.
    pub step_in: bool,
}

pub struct Session {
    tx: Sender<Cmd>,
    rx: Receiver<Out>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Session {
    pub fn start(setup: Setup, wake: Option<egui::Context>) -> Session {
        let (ctx, crx) = channel();
        let (otx, orx) = channel();
        let thread = std::thread::Builder::new()
            .name("script run".into())
            .stack_size(16 << 20)
            .spawn(move || {
                let tx = Sender::clone(&otx);
                let r = run(setup, crx, otx, wake.clone());
                let _ = tx.send(Out::Ended(r.err().map(|e| error_text(&e))));
                if let Some(w) = wake {
                    w.request_repaint();
                }
            })
            .ok();
        Session { tx: ctx, rx: orx, thread }
    }

    pub fn send(&self, c: Cmd) {
        let _ = self.tx.send(c);
    }

    pub fn poll(&self) -> Vec<Out> {
        self.rx.try_iter().collect()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Stop);
        // (not joined: a script stuck in a C call would hold the window; the thread ends at
        // its next instruction count)
        drop(self.thread.take());
    }
}

const STOPPED: &str = "the run was stopped";
/// A callback running this long without returning is stopped (an endless loop).
const BUDGET: Duration = Duration::from_secs(10);
const TICK: f64 = 1.0 / 30.0;

#[derive(Clone, Copy, PartialEq)]
enum Step {
    None,
    In,
    Over(usize),
    Out(usize),
}

/// What the hook and the loop share on the run's thread.
struct Dbg {
    breakpoints: HashMap<String, BTreeSet<usize>>,
    step: Step,
    pause: bool,
    stop: bool,
    /// Commands the hook read that the loop handles (an event sent while a script runs).
    later: Vec<Cmd>,
    busy_since: Option<Instant>,
    /// Tables shown, by their id (cleared when the script goes on).
    refs: Vec<Table>,
    /// The stack's frames' levels while stopped.
    levels: Vec<i64>,
    /// Functions of the API already said to do nothing here.
    noted: HashSet<String>,
}

struct Ctx {
    tx: Sender<Out>,
    rx: Receiver<Cmd>,
    dbg: RefCell<Dbg>,
    wake: Option<egui::Context>,
    /// Seconds since the start, and the clock.
    time: RefCell<(f64, f64)>,
    speed: RefCell<f64>,
    storage: RefCell<serde_json::Map<String, serde_json::Value>>,
    /// Each object's variables and texts set by the script, and what was done to its meshes.
    objects: RefCell<HashMap<i64, Vec<(String, String)>>>,
    state_changed: RefCell<bool>,
}

impl Ctx {
    fn send(&self, o: Out) {
        let _ = self.tx.send(o);
        if let Some(w) = &self.wake {
            w.request_repaint();
        }
    }

    fn line(&self, kind: LineKind, text: String, at: Option<(String, usize)>) {
        self.send(Out::Line { kind, text, at });
    }
}

/// The project file of a chunk name (`@scripts/x.lua`).
fn file_of(source: &str) -> Option<&str> {
    source.strip_prefix('@')
}

/// Where the script that called a Rust function is (`print` from scripts/x.lua line 4).
fn caller(lua: &Lua) -> Option<(String, usize)> {
    (1..6).find_map(|level| {
        lua.inspect_stack(level, |d| {
            let src = d.source();
            let f = file_of(src.source.as_deref()?)?.to_string();
            let line = d.current_line()?;
            (line > 0).then_some((f, line))
        })?
    })
}

fn texts(args: &MultiValue) -> String {
    args.iter().map(|v| match v {
        Value::String(s) => s.to_string_lossy().to_string(),
        other => plain(other),
    }).collect::<Vec<_>>().join("\t")
}

/// A value as `tostring` writes it (tables and functions by kind, no address).
fn plain(v: &Value) -> String {
    match v {
        Value::Nil => "nil".into(),
        Value::Boolean(b) => b.to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Number(n) => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                format!("{n:.1}")
            } else {
                format!("{n}")
            }
        }
        Value::String(s) => s.to_string_lossy().to_string(),
        Value::Table(_) => "table".into(),
        Value::Function(_) => "function".into(),
        // (the message alone: the handler that catches it adds where it happened)
        Value::Error(e) => match e.as_ref() {
            mlua::Error::CallbackError { cause, .. } => error_text(cause),
            other => error_text(other),
        },
        other => other.type_name().to_string(),
    }
}

fn show(v: &Value, refs: &mut Vec<Table>) -> Val {
    match v {
        Value::String(s) => {
            let t = s.to_string_lossy().to_string();
            let t = if t.chars().count() > 200 { format!("{}…", t.chars().take(200).collect::<String>()) } else { t };
            Val { text: format!("{t:?}"), ty: "string", table: None }
        }
        Value::Table(t) => {
            let n = t.clone().pairs::<Value, Value>().take(10_000).count();
            refs.push(t.clone());
            Val { text: format!("{{…}} {n}"), ty: "table", table: Some(refs.len() - 1) }
        }
        Value::Function(f) => {
            let info = f.info();
            let at = info.source.as_deref().and_then(file_of).map(|f| format!(" {f}:{}", info.line_defined.unwrap_or(0))).unwrap_or_default();
            Val { text: format!("function{at}"), ty: "function", table: None }
        }
        other => Val { text: plain(other), ty: other.type_name(), table: None },
    }
}

/// A key of a table as the debugger lists it.
fn key_text(k: &Value) -> String {
    match k {
        Value::String(s) => s.to_string_lossy().to_string(),
        Value::Integer(i) => format!("[{i}]"),
        other => format!("[{}]", plain(other)),
    }
}

fn arg_value(lua: &Lua, a: &Arg) -> mlua::Result<Value> {
    Ok(match a {
        Arg::Nil => Value::Nil,
        Arg::Bool(b) => Value::Boolean(*b),
        Arg::Num(n) if n.fract() == 0.0 && n.abs() < 9e15 => Value::Integer(*n as i64),
        Arg::Num(n) => Value::Number(*n),
        Arg::Str(s) => Value::String(lua.create_string(s)?),
    })
}

const SNAPSHOT: &str = r#"
local getinfo, getlocal, getupvalue = ...
return function()
  local frames = {}
  local level = 2
  while true do
    local info = getinfo(level, "Slnf")
    if not info then break end
    if info.what ~= "C" and info.source:sub(1, 1) == "@" then
      local locals, i = {}, 1
      while true do
        local name, value = getlocal(level, i)
        if not name then break end
        if name:sub(1, 1) ~= "(" then locals[#locals + 1] = { name = name, value = value } end
        i = i + 1
      end
      local ups, j = {}, 1
      while info.func do
        local name, value = getupvalue(info.func, j)
        if not name then break end
        if name ~= "_ENV" then ups[#ups + 1] = { name = name, value = value } end
        j = j + 1
      end
      local fname = info.name or (info.what == "main" and "(main chunk)") or ("function " .. info.short_src .. ":" .. info.linedefined)
      frames[#frames + 1] = { func = fname, source = info.source, line = info.currentline, locals = locals, ups = ups, level = level }
    end
    level = level + 1
  end
  return frames
end
"#;

const EVAL: &str = r#"
local getinfo, getlocal, setlocal, getupvalue, setupvalue = ...
return function(level, expr)
  local vars, where = {}, {}
  local info = getinfo(level, "f")
  if info and info.func then
    local j = 1
    while true do
      local n, v = getupvalue(info.func, j)
      if not n then break end
      if n ~= "_ENV" then vars[n] = v; where[n] = -j end
      j = j + 1
    end
  end
  local i = 1
  while true do
    local n, v = getlocal(level, i)
    if not n then break end
    if n:sub(1, 1) ~= "(" then vars[n] = v; where[n] = i end
    i = i + 1
  end
  local writes = {}
  local env = setmetatable({}, {
    __index = function(_, k) if where[k] then return vars[k] end return _G[k] end,
    __newindex = function(_, k, v) if where[k] then vars[k] = v; writes[k] = true else _G[k] = v end end,
  })
  local chunk, err = load("return " .. expr, "=console", "t", env)
  if not chunk then chunk, err = load(expr, "=console", "t", env) end
  if not chunk then return false, err end
  local res = table.pack(pcall(chunk))
  for k in pairs(writes) do
    local w = where[k]
    if w > 0 then setlocal(level, w, vars[k]) else setupvalue(info.func, -w, vars[k]) end
  end
  return table.unpack(res, 1, res.n)
end
"#;

/// The run, until it is stopped or the window closes.
fn run(setup: Setup, rx: Receiver<Cmd>, tx: Sender<Out>, wake: Option<egui::Context>) -> mlua::Result<()> {
    // (the debug library is the debugger's: taken before the scripts' globals lose it)
    let lua = unsafe { Lua::unsafe_new_with(StdLib::ALL_SAFE | StdLib::DEBUG, LuaOptions::new()) };
    let start_clock = setup.clock.rem_euclid(86400.0);
    let cx = Rc::new(Ctx {
        tx,
        rx,
        dbg: RefCell::new(Dbg { breakpoints: setup.breakpoints.clone(), step: if setup.step_in { Step::In } else { Step::None }, pause: false, stop: false, later: Vec::new(), busy_since: None, refs: Vec::new(), levels: Vec::new(), noted: HashSet::new() }),
        wake,
        time: RefCell::new((0.0, start_clock)),
        speed: RefCell::new(setup.speed.max(0.0)),
        storage: RefCell::new(Default::default()),
        objects: RefCell::new(HashMap::new()),
        state_changed: RefCell::new(true),
    });
    let g = lua.globals();
    let debug: Table = g.get("debug")?;
    let traceback: Function = debug.get("traceback")?;
    let snapshot: Function = lua.load(SNAPSHOT).set_name("=openMapper").call((debug.get::<Function>("getinfo")?, debug.get::<Function>("getlocal")?, debug.get::<Function>("getupvalue")?))?;
    let eval: Function = lua.load(EVAL).set_name("=openMapper").call((debug.get::<Function>("getinfo")?, debug.get::<Function>("getlocal")?, debug.get::<Function>("setlocal")?, debug.get::<Function>("getupvalue")?, debug.get::<Function>("setupvalue")?))?;
    sandbox(&lua, &setup.files)?;
    let omsi = api_table(&lua, &cx, &setup)?;
    g.set("omsi", omsi.clone())?;
    // print and the log lines, with the line they come from
    for (name, kind) in [("print", LineKind::Print)] {
        let c = cx.clone();
        g.set(name, lua.create_function(move |lua, args: MultiValue| {
            c.line(kind, texts(&args), caller(lua));
            Ok(())
        })?)?;
    }
    let report = {
        let c = cx.clone();
        lua.create_function(move |_, (where_, err): (String, Value)| {
            let text = plain(&err);
            let at = first_location(&text);
            c.line(LineKind::Error, format!("{where_}: {text}"), at);
            Ok(())
        })?
    };
    let note = {
        let c = cx.clone();
        lua.create_function(move |_, (what, text): (String, String)| {
            if c.dbg.borrow_mut().noted.insert(what.clone()) {
                c.line(LineKind::Note, format!("{what}: {text}"), None);
            }
            Ok(())
        })?
    };
    let now = {
        let c = cx.clone();
        lua.create_function(move |_, ()| Ok(c.time.borrow().0))?
    };
    let (fire, tick, counts): (Function, Function, Function) = lua.load(include_str!("prelude.lua")).set_name("=openMapper").call((omsi, traceback.clone(), report, note, now))?;

    // the debugger: a line hook that stops where asked, an instruction count that hears
    // Pause and Stop and ends an endless loop
    {
        let c = cx.clone();
        let snapshot = snapshot.clone();
        let eval = eval.clone();
        lua.set_hook(HookTriggers::new().every_line().every_nth_instruction(10_000), move |lua, d| {
            match d.event() {
                mlua::debug::DebugEvent::Count => {
                    for cmd in c.rx.try_iter().collect::<Vec<_>>() {
                        match cmd {
                            Cmd::Stop => c.dbg.borrow_mut().stop = true,
                            Cmd::Pause => c.dbg.borrow_mut().pause = true,
                            Cmd::Breakpoints(b) => c.dbg.borrow_mut().breakpoints = b,
                            other => c.dbg.borrow_mut().later.push(other),
                        }
                    }
                    let mut dbg = c.dbg.borrow_mut();
                    if dbg.stop {
                        return Err(mlua::Error::runtime(STOPPED));
                    }
                    if dbg.busy_since.is_some_and(|t| t.elapsed() > BUDGET) {
                        dbg.busy_since = None;
                        return Err(mlua::Error::runtime(format!("ran longer than {} s without returning: an endless loop? (the game would stop it too)", BUDGET.as_secs())));
                    }
                    Ok(VmState::Continue)
                }
                mlua::debug::DebugEvent::Line => {
                    let src = d.source();
                    let Some(file) = src.source.as_deref().and_then(file_of).map(str::to_string) else { return Ok(VmState::Continue) };
                    let line = d.current_line().unwrap_or(0);
                    let reason = {
                        let dbg = c.dbg.borrow();
                        if dbg.pause {
                            Some("paused")
                        } else if dbg.breakpoints.get(&file).is_some_and(|b| b.contains(&line)) {
                            Some("breakpoint")
                        } else {
                            match dbg.step {
                                Step::None => None,
                                Step::In => Some("step"),
                                Step::Over(d0) => (depth(lua) <= d0).then_some("step"),
                                Step::Out(d0) => (depth(lua) < d0).then_some("step"),
                            }
                        }
                    };
                    match reason {
                        Some(r) => stopped(lua, &c, &snapshot, &eval, r, file, line).map(|_| VmState::Continue),
                        None => Ok(VmState::Continue),
                    }
                }
                _ => Ok(VmState::Continue),
            }
        })?;
    }

    // the main chunk
    cx.line(LineKind::Note, format!("{} {}", crate::i18n::tr("Test run of", "Тестовый запуск"), setup.entry), None);
    let entry_text = setup.files.iter().find(|f| f.0 == setup.entry).map(|f| f.1.clone()).unwrap_or_default();
    cx.dbg.borrow_mut().busy_since = Some(Instant::now());
    let r = lua.load(entry_text).set_name(format!("@{}", setup.entry)).exec();
    cx.dbg.borrow_mut().busy_since = None;
    if let Err(e) = r {
        let text = error_text(&e);
        if text.contains(STOPPED) {
            return Ok(());
        }
        let at = first_location(&text);
        cx.line(LineKind::Error, text, at);
    }
    let call = |f: &Function, args: MultiValue| -> bool {
        cx.dbg.borrow_mut().busy_since = Some(Instant::now());
        let r = f.call::<()>(args);
        cx.dbg.borrow_mut().busy_since = None;
        match r {
            Err(e) if error_text(&e).contains(STOPPED) => false,
            Err(e) => {
                let text = error_text(&e);
                let at = first_location(&text);
                cx.line(LineKind::Error, text, at);
                true
            }
            Ok(()) => true,
        }
    };
    let fire_event = |name: &str, args: Vec<Value>| -> bool {
        let mut mv = MultiValue::new();
        mv.push_back(Value::String(lua.create_string(name).unwrap()));
        for a in args {
            mv.push_back(a);
        }
        call(&fire, mv)
    };
    if !fire_event("start", vec![]) {
        return Ok(());
    }
    for o in setup.objects.iter().take(500) {
        let args = vec![Value::Integer(o.id), Value::String(lua.create_string(&o.file)?), Value::Number(o.pos[0]), Value::Number(o.pos[1]), Value::Number(o.pos[2]), Value::Number(o.heading)];
        if !fire_event("object_init", args) {
            return Ok(());
        }
    }

    let mut running = true;
    let mut last_report = Instant::now() - Duration::from_secs(1);
    let mut next_tick = Instant::now();
    let report_listening = |cx: &Ctx| -> mlua::Result<()> {
        let (events, timers): (Table, usize) = counts.call(())?;
        let mut ev: Vec<(String, usize)> = events.pairs::<String, usize>().flatten().collect();
        ev.sort();
        cx.send(Out::Listening { events: ev, timers });
        Ok(())
    };
    report_listening(&cx)?;
    loop {
        // commands: those the hook kept, then the new ones (waiting while time stands still)
        let mut cmds = std::mem::take(&mut cx.dbg.borrow_mut().later);
        let wait = if running { next_tick.saturating_duration_since(Instant::now()) } else { Duration::from_millis(250) };
        if cmds.is_empty() {
            match cx.rx.recv_timeout(wait) {
                Ok(c) => cmds.push(c),
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
        cmds.extend(cx.rx.try_iter());
        for cmd in cmds {
            match cmd {
                Cmd::Stop => return Ok(()),
                Cmd::Continue => running = true,
                Cmd::Pause => running = false,
                Cmd::StepIn | Cmd::StepOver => cx.dbg.borrow_mut().step = Step::In,
                Cmd::StepOut => {}
                Cmd::Breakpoints(b) => cx.dbg.borrow_mut().breakpoints = b,
                Cmd::Speed(s) => *cx.speed.borrow_mut() = s.max(0.0),
                Cmd::SetClock(c) => {
                    cx.time.borrow_mut().1 = c.rem_euclid(86400.0);
                    let (h, m) = ((c / 3600.0) as i64 % 24, (c / 60.0) as i64 % 60);
                    if !fire_event("minute", vec![Value::Integer(h), Value::Integer(m)]) {
                        return Ok(());
                    }
                }
                Cmd::Fire(name, args) => {
                    let vals = args.iter().map(|a| arg_value(&lua, a)).collect::<mlua::Result<Vec<_>>>()?;
                    cx.line(LineKind::Note, format!("-> {name}({})", args.iter().map(|a| match a { Arg::Nil => "nil".into(), Arg::Bool(b) => b.to_string(), Arg::Num(n) => n.to_string(), Arg::Str(s) => format!("{s:?}") }).collect::<Vec<_>>().join(", ")), None);
                    if !fire_event(&name, vals) {
                        return Ok(());
                    }
                    report_listening(&cx)?;
                }
                Cmd::Eval(expr, _) => {
                    let r = eval_global(&lua, &expr);
                    let mut dbg = cx.dbg.borrow_mut();
                    let text = match r {
                        Ok(vals) => vals.iter().map(|v| show(v, &mut dbg.refs).text).collect::<Vec<_>>().join(", "),
                        Err(e) => error_text(&e),
                    };
                    drop(dbg);
                    cx.line(LineKind::Result, format!("> {expr}\n{text}"), None);
                }
                Cmd::Expand(id) => {
                    let items = expand(&cx, id);
                    cx.send(Out::Expanded { id, items });
                }
            }
        }
        if !running {
            if last_report.elapsed() > Duration::from_millis(500) {
                let (time, clock) = *cx.time.borrow();
                cx.send(Out::Clock { clock, time, running });
                last_report = Instant::now();
            }
            continue;
        }
        if Instant::now() < next_tick {
            continue;
        }
        next_tick += Duration::from_secs_f64(TICK);
        if next_tick < Instant::now() {
            next_tick = Instant::now() + Duration::from_secs_f64(TICK);
        }
        // a frame of the game
        let dt = TICK * *cx.speed.borrow();
        let (old_clock, new_clock) = {
            let mut t = cx.time.borrow_mut();
            t.0 += dt;
            let old = t.1;
            t.1 = (t.1 + dt).rem_euclid(86400.0);
            (old, t.1)
        };
        if dt > 0.0 {
            if !fire_event("frame", vec![Value::Number(dt)]) {
                return Ok(());
            }
            for o in setup.objects.iter().take(500) {
                if !fire_event("object_frame", vec![Value::Integer(o.id), Value::Number(dt)]) {
                    return Ok(());
                }
            }
            if !call(&tick, MultiValue::new()) {
                return Ok(());
            }
            let crossed = |unit: f64| (old_clock / unit).floor() != (new_clock / unit).floor() || new_clock < old_clock;
            if crossed(60.0) {
                let (h, m) = ((new_clock / 3600.0) as i64 % 24, (new_clock / 60.0) as i64 % 60);
                if !fire_event("minute", vec![Value::Integer(h), Value::Integer(m)]) {
                    return Ok(());
                }
                if crossed(3600.0) && !fire_event("hour", vec![Value::Integer(h)]) {
                    return Ok(());
                }
                if new_clock < old_clock {
                    let d = chrono_today(cx.time.borrow().0 + start_clock);
                    if !fire_event("day", vec![Value::Integer(d.0), Value::Integer(d.1), Value::Integer(d.2)]) {
                        return Ok(());
                    }
                }
            }
        }
        if last_report.elapsed() > Duration::from_millis(250) {
            let (time, clock) = *cx.time.borrow();
            cx.send(Out::Clock { clock, time, running });
            report_listening(&cx)?;
            if std::mem::take(&mut *cx.state_changed.borrow_mut()) {
                send_state(&cx);
            }
            last_report = Instant::now();
        }
    }
}

/// The date of the run: today, moved on by the days the clock passed midnight.
fn chrono_today(seconds_since_start_midnight: f64) -> (i64, i64, i64) {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let days = now / 86400 + (seconds_since_start_midnight / 86400.0).floor() as i64;
    civil(days)
}

/// Year, month, day of a day count since 1970 (H. Hinnant's algorithm).
fn civil(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn depth(lua: &Lua) -> usize {
    let mut n = 0;
    while lua.inspect_stack(n, |_| ()).is_some() {
        n += 1;
    }
    n
}

fn error_text(e: &mlua::Error) -> String {
    match e {
        mlua::Error::CallbackError { traceback, cause } => format!("{}\n{traceback}", error_text(cause)),
        mlua::Error::RuntimeError(s) | mlua::Error::SyntaxError { message: s, .. } => s.clone(),
        mlua::Error::WithContext { cause, .. } => error_text(cause),
        other => other.to_string(),
    }
}

/// The first `scripts/x.lua:12:` in an error's text.
fn first_location(text: &str) -> Option<(String, usize)> {
    for word in text.split(|c: char| c.is_whitespace() || c == '\'' || c == '"') {
        let mut parts = word.split(':');
        let (Some(f), Some(l)) = (parts.next(), parts.next()) else { continue };
        if f.ends_with(".lua") && !f.starts_with('[') {
            if let Ok(n) = l.parse() {
                return Some((f.trim_start_matches('@').to_string(), n));
            }
        }
    }
    None
}

/// Stopped in a script: the stack sent, then commands until it may go on.
fn stopped(lua: &Lua, c: &Rc<Ctx>, snapshot: &Function, eval: &Function, reason: &str, file: String, line: usize) -> mlua::Result<()> {
    let busy = c.dbg.borrow().busy_since.map(|t| t.elapsed());
    let frames: Table = snapshot.call(())?;
    let mut stack = Vec::new();
    {
        let mut dbg = c.dbg.borrow_mut();
        dbg.refs.clear();
        dbg.levels.clear();
        dbg.pause = false;
        for f in frames.sequence_values::<Table>().flatten() {
            let vars = |key: &str, refs: &mut Vec<Table>| -> Vec<(String, Val)> {
                f.get::<Table>(key).map(|t| t.sequence_values::<Table>().flatten().map(|v| (v.get::<String>("name").unwrap_or_default(), show(&v.get::<Value>("value").unwrap_or(Value::Nil), refs))).collect()).unwrap_or_default()
            };
            let locals = vars("locals", &mut dbg.refs);
            let upvalues = vars("ups", &mut dbg.refs);
            let level: i64 = f.get("level").unwrap_or(0);
            dbg.levels.push(level);
            let src: String = f.get("source").unwrap_or_default();
            stack.push(Frame { func: f.get("func").unwrap_or_default(), file: file_of(&src).unwrap_or(&src).to_string(), line: f.get::<i64>("line").unwrap_or(0).max(0) as usize, locals, upvalues });
        }
    }
    c.send(Out::Paused { reason: reason.to_string(), file, line, stack });
    loop {
        let cmd = match c.rx.recv() {
            Ok(cmd) => cmd,
            Err(_) => return Err(mlua::Error::runtime(STOPPED)),
        };
        let d = depth(lua);
        match cmd {
            Cmd::Continue => {
                c.dbg.borrow_mut().step = Step::None;
                break;
            }
            Cmd::StepIn => {
                c.dbg.borrow_mut().step = Step::In;
                break;
            }
            Cmd::StepOver => {
                c.dbg.borrow_mut().step = Step::Over(d);
                break;
            }
            Cmd::StepOut => {
                c.dbg.borrow_mut().step = Step::Out(d);
                break;
            }
            Cmd::Stop => {
                c.dbg.borrow_mut().stop = true;
                return Err(mlua::Error::runtime(STOPPED));
            }
            Cmd::Pause => {}
            Cmd::Breakpoints(b) => c.dbg.borrow_mut().breakpoints = b,
            Cmd::Eval(expr, frame) => {
                let level = frame.and_then(|i| c.dbg.borrow().levels.get(i).copied()).unwrap_or(2);
                let r: mlua::Result<MultiValue> = eval.call((level, expr.clone()));
                let mut dbg = c.dbg.borrow_mut();
                let text = match r {
                    Ok(mv) => {
                        let mut it = mv.into_iter();
                        match it.next() {
                            Some(Value::Boolean(true)) => {
                                let vals: Vec<String> = it.map(|v| show(&v, &mut dbg.refs).text).collect();
                                if vals.is_empty() { "nil".into() } else { vals.join(", ") }
                            }
                            _ => it.next().map(|v| plain(&v)).unwrap_or_else(|| "error".into()),
                        }
                    }
                    Err(e) => error_text(&e),
                };
                drop(dbg);
                c.line(LineKind::Result, format!("> {expr}\n{text}"), None);
            }
            Cmd::Expand(id) => {
                let items = expand(c, id);
                c.send(Out::Expanded { id, items });
            }
            other => c.dbg.borrow_mut().later.push(other),
        }
    }
    // (the time stopped does not count against the callback's budget)
    if let Some(b) = busy {
        c.dbg.borrow_mut().busy_since = Some(Instant::now() - b);
    }
    c.dbg.borrow_mut().refs.clear();
    c.send(Out::Resumed);
    let _ = lua;
    Ok(())
}

fn expand(c: &Ctx, id: usize) -> Vec<(String, Val)> {
    let Some(t) = c.dbg.borrow().refs.get(id).cloned() else { return Vec::new() };
    let mut items: Vec<(Value, Value)> = t.pairs::<Value, Value>().flatten().take(500).collect();
    items.sort_by(|a, b| match (&a.0, &b.0) {
        (Value::Integer(x), Value::Integer(y)) => x.cmp(y),
        (Value::Integer(_), _) => std::cmp::Ordering::Less,
        (_, Value::Integer(_)) => std::cmp::Ordering::Greater,
        (x, y) => key_text(x).cmp(&key_text(y)),
    });
    let mut dbg = c.dbg.borrow_mut();
    items.into_iter().map(|(k, v)| (key_text(&k), show(&v, &mut dbg.refs))).collect()
}

fn eval_global(lua: &Lua, expr: &str) -> mlua::Result<Vec<Value>> {
    let f = match lua.load(format!("return {expr}")).set_name("=console").into_function() {
        Ok(f) => f,
        Err(_) => lua.load(expr).set_name("=console").into_function()?,
    };
    Ok(f.call::<MultiValue>(())?.into_iter().collect())
}

fn send_state(c: &Ctx) {
    let storage = c.storage.borrow().iter().map(|(k, v)| (k.clone(), v.to_string())).collect();
    let mut objects: Vec<(i64, Vec<(String, String)>)> = c.objects.borrow().iter().map(|(k, v)| (*k, v.clone())).collect();
    objects.sort_by_key(|o| o.0);
    c.send(Out::State { storage, objects });
}

/// The libraries a plugin has in the game: `os` only for the clock, `load` of text only, no
/// files, and `require` of the project's own modules.
fn sandbox(lua: &Lua, files: &[(String, String)]) -> mlua::Result<()> {
    let g = lua.globals();
    for name in ["dofile", "loadfile", "debug", "io", "collectgarbage"] {
        g.set(name, Value::Nil)?;
    }
    let os: Table = g.get("os")?;
    for name in ["execute", "exit", "getenv", "remove", "rename", "tmpname", "setlocale"] {
        os.set(name, Value::Nil)?;
    }
    let string: Table = g.get("string")?;
    string.set("dump", Value::Nil)?;
    let load: Function = g.get("load")?;
    g.set("load", lua.create_function(move |lua, (chunk, name, _mode, env): (Value, Option<String>, Option<String>, Option<Value>)| {
        let mut args = MultiValue::new();
        args.push_back(chunk);
        args.push_back(match name {
            Some(n) => Value::String(lua.create_string(n)?),
            None => Value::Nil,
        });
        args.push_back(Value::String(lua.create_string("t")?));
        if let Some(e) = env {
            args.push_back(e);
        }
        load.call::<MultiValue>(args)
    })?)?;
    let package: Table = g.get("package")?;
    package.set("path", "")?;
    package.set("cpath", "")?;
    package.set("loadlib", Value::Nil)?;
    let files: HashMap<String, String> = files.iter().cloned().collect();
    let searcher = lua.create_function(move |lua, name: String| {
        let base = name.replace('.', "/");
        for rel in [format!("{base}.lua"), format!("{base}/init.lua")] {
            if let Some(text) = files.get(&rel) {
                let f = lua.load(text.as_str()).set_name(format!("@{rel}")).into_function()?;
                return Ok((Value::Function(f), Value::String(lua.create_string(&rel)?)));
            }
        }
        Ok((Value::String(lua.create_string(format!("\n\tno file '{base}.lua' in the project"))?), Value::Nil))
    })?;
    let searchers: Table = package.get("searchers")?;
    let preload: Function = searchers.get(1)?;
    let list = lua.create_table()?;
    list.push(preload)?;
    list.push(searcher)?;
    package.set("searchers", list)?;
    Ok(())
}

/// The `omsi` table: a stand-in for every function of the game's API, then the ones that
/// work without a game.
fn api_table(lua: &Lua, cx: &Rc<Ctx>, setup: &Setup) -> mlua::Result<Table> {
    let omsi = lua.create_table()?;
    let api = api();
    let table_of = |name: &str| -> mlua::Result<(Table, String)> {
        let mut t = omsi.clone();
        let mut parts: Vec<&str> = name.split('.').collect();
        let last = parts.pop().unwrap_or_default().to_string();
        for p in parts {
            t = match t.get::<Value>(p)? {
                Value::Table(x) => x,
                _ => {
                    let x = lua.create_table()?;
                    t.set(p, x.clone())?;
                    x
                }
            };
        }
        Ok((t, last))
    };
    for f in &api.functions {
        let (t, last) = table_of(&f.name)?;
        let c = cx.clone();
        let fname = f.name.clone();
        let params = f.params.clone();
        let change = f.is_change();
        t.set(last, lua.create_function(move |lua, args: MultiValue| {
            check_args(&fname, &params, &args)?;
            if c.dbg.borrow_mut().noted.insert(fname.clone()) {
                let why = if change { crate::i18n::tr("does nothing in the test run (no game): it gives false", "в тестовом запуске ничего не делает (игры нет): возвращает false") } else { crate::i18n::tr("has nothing to read in the test run (no game): it gives nil", "в тестовом запуске нечего читать (игры нет): возвращает nil") };
                c.line(LineKind::Note, format!("omsi.{fname}: {why}"), caller(lua));
            }
            let mut out = MultiValue::new();
            if change {
                out.push_back(Value::Boolean(false));
                out.push_back(Value::String(lua.create_string("not in openMapper's test run")?));
            }
            Ok(out)
        })?)?;
    }
    // (a function that works here still checks its arguments as the game does)
    let set = |name: &str, f: Function| -> mlua::Result<()> {
        let (t, last) = table_of(name)?;
        let params = api.function(name).map(|x| x.params.clone()).unwrap_or_default();
        let fname = name.to_string();
        t.set(last, lua.create_function(move |_, args: MultiValue| {
            check_args(&fname, &params, &args)?;
            f.call::<MultiValue>(args)
        })?)
    };
    // the log and the screen
    for (name, kind) in [("log", LineKind::Print), ("debug", LineKind::Print), ("warn", LineKind::Warn), ("error", LineKind::Error)] {
        let c = cx.clone();
        set(name, lua.create_function(move |lua, args: MultiValue| {
            c.line(kind, texts(&args), caller(lua));
            Ok(())
        })?)?;
    }
    {
        let c = cx.clone();
        set("message", lua.create_function(move |lua, (text, seconds): (Value, Option<f64>)| {
            c.line(LineKind::Screen, format!("{} ({} s)", plain(&text), seconds.unwrap_or(5.0)), caller(lua));
            Ok(())
        })?)?;
    }
    {
        let c = cx.clone();
        set("ui.toast", lua.create_function(move |lua, (text, opts): (Value, Option<Table>)| {
            let title = opts.and_then(|o| o.get::<String>("title").ok()).map(|t| format!("{t}: ")).unwrap_or_default();
            c.line(LineKind::Screen, format!("[toast] {title}{}", plain(&text)), caller(lua));
            Ok(true)
        })?)?;
    }
    // the map
    let folder = setup.map_folder.clone();
    set("map.name", lua.create_function(move |_, ()| Ok(folder.clone()))?)?;
    {
        let (folder, friendly, path, lht) = (setup.map_folder.clone(), setup.friendly.clone(), setup.map_path.clone(), setup.lht);
        set("map.info", lua.create_function(move |lua, ()| {
            let t = lua.create_table()?;
            t.set("name", folder.clone())?;
            t.set("friendly_name", friendly.clone())?;
            t.set("path", path.clone())?;
            t.set("left_hand_traffic", lht)?;
            t.set("tile_size", 300.0)?;
            Ok(t)
        })?)?;
    }
    {
        let ground = setup.ground.clone();
        set("map.ground", lua.create_function(move |_, (x, y): (f64, f64)| Ok(ground(x, y)))?)?;
    }
    {
        let all = setup.all_objects.clone();
        set("map.objects_near", lua.create_function(move |lua, (x, y, r): (f64, f64, Option<f64>)| {
            let r = r.unwrap_or(50.0);
            let out = lua.create_table()?;
            for (id, file, p) in all.iter().filter(|o| (o.2[0] - x).hypot(o.2[1] - y) <= r).take(1000) {
                let t = lua.create_table()?;
                t.set("id", *id)?;
                t.set("file", file.clone())?;
                t.set("x", p[0])?;
                t.set("y", p[1])?;
                t.set("z", p[2])?;
                out.push(t)?;
            }
            Ok(out)
        })?)?;
    }
    // the clock
    {
        let c = cx.clone();
        set("time", lua.create_function(move |_, ()| Ok(c.time.borrow().0))?)?;
        let c = cx.clone();
        set("world.time", lua.create_function(move |_, ()| Ok(c.time.borrow().1))?)?;
        let c = cx.clone();
        set("clock", lua.create_function(move |_, ()| {
            let s = c.time.borrow().1 as i64;
            Ok(format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60))
        })?)?;
        let c = cx.clone();
        set("world.set_time", lua.create_function(move |_, t: Value| {
            let secs = match &t {
                Value::Integer(i) => Some(*i as f64),
                Value::Number(n) => Some(*n),
                Value::String(s) => {
                    let s = s.to_string_lossy();
                    let p: Vec<f64> = s.split(':').filter_map(|x| x.trim().parse().ok()).collect();
                    (p.len() >= 2).then(|| p[0] * 3600.0 + p[1] * 60.0 + p.get(2).copied().unwrap_or(0.0))
                }
                _ => None,
            };
            match secs {
                Some(s) => {
                    c.time.borrow_mut().1 = s.rem_euclid(86400.0);
                    Ok((true, None))
                }
                None => Ok((false, Some("a time is \"HH:MM\" or seconds since midnight".to_string()))),
            }
        })?)?;
        let c = cx.clone();
        set("world.time_speed", lua.create_function(move |_, ()| Ok(*c.speed.borrow()))?)?;
        let c = cx.clone();
        set("world.set_time_speed", lua.create_function(move |_, s: f64| {
            *c.speed.borrow_mut() = s.clamp(0.0, 1000.0);
            Ok(true)
        })?)?;
        set("world.paused", lua.create_function(|_, ()| Ok(false))?)?;
        let c = cx.clone();
        set("world.date", lua.create_function(move |lua, ()| {
            let (y, m, d) = chrono_today(c.time.borrow().0);
            let t = lua.create_table()?;
            t.set("year", y)?;
            t.set("month", m)?;
            t.set("day", d)?;
            Ok(t)
        })?)?;
        let c = cx.clone();
        let (folder, path) = (setup.map_folder.clone(), setup.map_path.clone());
        let info = lua.create_function(move |lua, ()| {
            let t = lua.create_table()?;
            let (time, clock) = *c.time.borrow();
            let (y, _, d) = chrono_today(time);
            t.set("map", folder.clone())?;
            t.set("map_path", path.clone())?;
            t.set("clock", clock)?;
            t.set("day", d)?;
            t.set("year", y)?;
            t.set("view", "free")?;
            t.set("paused", false)?;
            t.set("on_foot", false)?;
            t.set("multiplayer", false)?;
            t.set("traffic", 0)?;
            t.set("speed", 0.0)?;
            t.set("delay", 0.0)?;
            t.set("version", format!("openMapper test run (API {})", api.version))?;
            Ok(t)
        })?;
        set("info", info.clone())?;
        set("info_value", lua.create_function(move |_, key: String| info.call::<Table>(())?.get::<Value>(key))?)?;
    }
    // the game and the plugin
    set("game.has", lua.create_function(|_, name: String| Ok(api.function(&name).is_some()))?)?;
    set("game.version", lua.create_function(|_, ()| Ok(api.version.clone()))?)?;
    set("game.platform", lua.create_function(|_, ()| Ok("openMapper"))?)?;
    {
        let name = crate::script::project::plugin_name(&setup.map_folder);
        set("plugin.name", lua.create_function(move |_, ()| Ok(name.clone()))?)?;
    }
    // storage, kept for the run
    {
        let c = cx.clone();
        set("storage.get", lua.create_function(move |lua, key: String| match c.storage.borrow().get(&key) {
            Some(v) => lua.to_value(v),
            None => Ok(Value::Nil),
        })?)?;
        let c = cx.clone();
        set("storage.set", lua.create_function(move |lua, (key, v): (String, Value)| {
            if v.is_nil() {
                c.storage.borrow_mut().remove(&key);
            } else {
                c.storage.borrow_mut().insert(key, lua.from_value(v)?);
            }
            *c.state_changed.borrow_mut() = true;
            Ok(())
        })?)?;
    }
    // JSON
    set("json.encode", lua.create_function(|lua, (v, pretty): (Value, Option<bool>)| {
        let j: serde_json::Value = lua.from_value(v)?;
        Ok(if pretty.unwrap_or(false) { serde_json::to_string_pretty(&j) } else { serde_json::to_string(&j) }.unwrap_or_default())
    })?)?;
    set("json.decode", lua.create_function(|lua, text: String| match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(j) => Ok((lua.to_value(&j)?, Value::Nil)),
        Err(e) => Ok((Value::Nil, Value::String(lua.create_string(e.to_string())?))),
    })?)?;
    // real time and chance
    let started = Instant::now();
    set("util.ms", lua.create_function(move |_, ()| Ok(started.elapsed().as_secs_f64() * 1000.0))?)?;
    set("util.now", lua.create_function(|_, ()| Ok(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)))?)?;
    {
        let seed = RefCell::new(0x9E37_79B9_7F4A_7C15u64 ^ started.elapsed().as_nanos() as u64 ^ std::process::id() as u64);
        set("util.random", lua.create_function(move |_, (lo, hi): (Option<f64>, Option<f64>)| {
            let mut s = seed.borrow_mut();
            *s ^= *s << 13;
            *s ^= *s >> 7;
            *s ^= *s << 17;
            let u = (*s >> 11) as f64 / (1u64 << 53) as f64;
            Ok(match (lo, hi) {
                (Some(a), Some(b)) => (a + (u * (b - a + 1.0)).floor()).min(b),
                (Some(b), None) => (1.0 + (u * b).floor()).min(b),
                _ => u,
            })
        })?)?;
    }
    // the objects a script of objects drives: their variables and texts, kept for the run
    {
        let ids: HashSet<i64> = setup.objects.iter().map(|o| o.id).collect();
        let check = move |id: i64| -> mlua::Result<()> {
            if ids.contains(&id) { Ok(()) } else { Err(mlua::Error::runtime(format!("omsi.object: {id} is not an object this script drives"))) }
        };
        let put = |c: &Ctx, id: i64, key: String, v: String| {
            let mut o = c.objects.borrow_mut();
            let e = o.entry(id).or_default();
            match e.iter_mut().find(|x| x.0 == key) {
                Some(x) => x.1 = v,
                None => e.push((key, v)),
            }
            *c.state_changed.borrow_mut() = true;
        };
        let get = |c: &Ctx, id: i64, key: &str| -> Option<String> { c.objects.borrow().get(&id)?.iter().find(|x| x.0 == key).map(|x| x.1.clone()) };
        let (c, k) = (cx.clone(), check.clone());
        set("object.var", lua.create_function(move |_, (id, name): (i64, String)| {
            k(id)?;
            Ok(get(&c, id, &name).and_then(|v| v.parse::<f64>().ok()))
        })?)?;
        let (c, k) = (cx.clone(), check.clone());
        set("object.set_var", lua.create_function(move |_, (id, name, v): (i64, String, f64)| {
            k(id)?;
            put(&c, id, name, v.to_string());
            Ok(())
        })?)?;
        let (c, k) = (cx.clone(), check.clone());
        set("object.str", lua.create_function(move |_, (id, name): (i64, String)| {
            k(id)?;
            Ok(get(&c, id, &format!("${name}")))
        })?)?;
        let (c, k) = (cx.clone(), check.clone());
        set("object.set_str", lua.create_function(move |_, (id, name, v): (i64, String, String)| {
            k(id)?;
            put(&c, id, format!("${name}"), v);
            Ok(())
        })?)?;
        let (c, k) = (cx.clone(), check.clone());
        set("object.set_visible", lua.create_function(move |_, (id, mesh, v): (i64, String, Option<bool>)| {
            k(id)?;
            put(&c, id, format!("visible {mesh}"), v.map(|b| b.to_string()).unwrap_or_else(|| "as the object says".into()));
            Ok(())
        })?)?;
        let (c, k) = (cx.clone(), check.clone());
        set("object.set_mesh", lua.create_function(move |_, (id, mesh, t): (i64, String, Option<Table>)| {
            k(id)?;
            let text = t.map(|t| ["x", "y", "z", "rx", "ry", "rz", "sx", "sy", "sz"].iter().filter_map(|key| t.get::<f64>(*key).ok().map(|v| format!("{key}={v:.2}"))).collect::<Vec<_>>().join(" ")).unwrap_or_default();
            put(&c, id, format!("mesh {mesh}"), text);
            Ok(())
        })?)?;
        let objs = setup.objects.clone();
        let k = check;
        set("object.info", lua.create_function(move |lua, id: i64| {
            k(id)?;
            let o = objs.iter().find(|o| o.id == id).expect("checked");
            let t = lua.create_table()?;
            t.set("id", id)?;
            t.set("type", o.file.clone())?;
            t.set("x", o.pos[0])?;
            t.set("y", o.pos[1])?;
            t.set("z", o.pos[2])?;
            t.set("heading", o.heading)?;
            Ok(t)
        })?)?;
        let objs = setup.objects.clone();
        set("object.list", lua.create_function(move |lua, ()| lua.create_sequence_from(objs.iter().map(|o| o.id)))?)?;
    }
    Ok(omsi)
}

/// The game's own check of an argument: `omsi.var: argument 1 (name) must be a string, not table`.
fn check_args(fname: &str, params: &[crate::script::api::Param], args: &MultiValue) -> mlua::Result<()> {
    for (i, p) in params.iter().enumerate() {
        if p.name == "..." || p.ty == "any" {
            break;
        }
        let v = args.get(i).unwrap_or(&Value::Nil);
        if v.is_nil() && p.optional {
            continue;
        }
        let ok = match p.ty.as_str() {
            "number" => matches!(v, Value::Number(_) | Value::Integer(_)),
            "integer" => matches!(v, Value::Integer(_)) || matches!(v, Value::Number(n) if n.fract() == 0.0),
            "string" => matches!(v, Value::String(_)) || matches!(v, Value::Number(_) | Value::Integer(_)),
            "table" => matches!(v, Value::Table(_)),
            "function" => matches!(v, Value::Function(_)),
            "bool" | "boolean" => matches!(v, Value::Boolean(_)),
            _ => true,
        };
        if !ok {
            return Err(mlua::Error::runtime(format!("omsi.{fname}: argument {} ({}) must be a {}, not {}", i + 1, p.name, p.ty, v.type_name())));
        }
    }
    Ok(())
}

/// Sensible values to send with an event by hand (`stop_arrive`: a stop's name, id, number, delay).
pub fn default_args(event: &crate::script::api::Event, clock: f64) -> Vec<String> {
    event
        .args
        .iter()
        .map(|a| {
            match a.as_str() {
                "name" | "terminus" => "\"Hauptbahnhof\"".into(),
                "hour" => ((clock / 3600.0) as i64 % 24).to_string(),
                "minute" => ((clock / 60.0) as i64 % 60).to_string(),
                "dt" => "0.033".into(),
                "id" | "number" | "trip" | "count" | "door" | "stop_id" | "x" | "y" | "z" | "heading" => "1".into(),
                "delay" | "old" | "speed_kmh" | "energy_kj" => "0".into(),
                "open" | "down" | "on" | "focused" => "true".into(),
                "line" => "\"1\"".into(),
                "tour" => "\"1\"".into(),
                "year" | "month" | "day" => {
                    let (y, m, d) = chrono_today(0.0);
                    match a.as_str() { "year" => y, "month" => m, _ => d }.to_string()
                }
                _ => "nil".into(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(files: &[(&str, &str)], entry: &str) -> Setup {
        Setup {
            files: files.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
            entry: entry.into(),
            map_folder: "Test".into(),
            friendly: "Test town".into(),
            map_path: String::new(),
            lht: false,
            ground: Arc::new(|x, _| Some(x * 0.5)),
            objects: vec![Obj { id: 7, file: "gate.sco".into(), pos: [1.0, 2.0, 3.0], heading: 90.0 }],
            all_objects: Arc::new(vec![]),
            clock: 8.0 * 3600.0 - 0.5,
            speed: 1.0,
            breakpoints: HashMap::new(),
            step_in: false,
        }
    }

    /// What comes out until `until` says it is enough (or 5 s pass).
    fn collect(s: &Session, until: impl Fn(&[Out]) -> bool) -> Vec<Out> {
        let mut all = Vec::new();
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(5) {
            all.extend(s.poll());
            if until(&all) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        all
    }

    fn lines(o: &[Out]) -> Vec<String> {
        o.iter().filter_map(|o| if let Out::Line { text, .. } = o { Some(text.clone()) } else { None }).collect()
    }

    #[test]
    fn a_project_runs_with_its_modules_events_and_timers() {
        let main = "local u = require(\"modules.util\")\nprint(u.twice(21), omsi.map.name(), omsi.map.ground(4, 0))\nomsi.on(\"hour\", function(h) print(\"hour\", h) end)\nomsi.after(0.1, function() print(\"later\", omsi.fmt.delay(150)) end)\nfunction on_stop_arrive(name, id) print(\"at\", name, id) end\n";
        let s = Session::start(setup(&[("main.lua", main), ("modules/util.lua", "return { twice = function(x) return x * 2 end }")], "main.lua"), None);
        let out = collect(&s, |o| lines(o).iter().any(|l| l.starts_with("hour")) && lines(o).iter().any(|l| l.starts_with("later")));
        let l = lines(&out);
        assert!(l.contains(&"42\tTest\t2.0".to_string()), "{l:?}");
        assert!(l.contains(&"hour\t8".to_string()), "{l:?}");
        assert!(l.contains(&"later\t+2:30".to_string()), "{l:?}");
        s.send(Cmd::Fire("stop_arrive".into(), vec![Arg::Str("Markt".into()), Arg::Num(3.0)]));
        let out = collect(&s, |o| lines(o).iter().any(|l| l.starts_with("at")));
        assert!(lines(&out).contains(&"at\tMarkt\t3".to_string()));
    }

    #[test]
    fn errors_name_their_line_and_the_game_s_functions_check_their_arguments() {
        let main = "omsi.on(\"start\", function() local t = nil; return t.x end)\nomsi.on(\"start\", function() omsi.map.ground(\"a\") end)\nprint(omsi.bus.state(), omsi.weather.set({}))\nprint(dofile, io, os.execute, debug)\n";
        let s = Session::start(setup(&[("scripts/a.lua", main)], "scripts/a.lua"), None);
        let out = collect(&s, |o| o.iter().filter(|o| matches!(o, Out::Line { kind: LineKind::Error, .. })).count() >= 2);
        let errs: Vec<&Out> = out.iter().filter(|o| matches!(o, Out::Line { kind: LineKind::Error, .. })).collect();
        assert!(errs.iter().any(|o| matches!(o, Out::Line { at: Some((f, 1)), .. } if f == "scripts/a.lua")), "{out:?}");
        assert!(lines(&out).iter().any(|l| l.contains("omsi.map.ground: argument 1 (x) must be a number, not string")), "{:?}", lines(&out));
        assert!(lines(&out).contains(&"nil\tfalse\tnot in openMapper's test run".to_string()), "{:?}", lines(&out));
        assert!(lines(&out).contains(&"nil\tnil\tnil\tnil".to_string()));
    }

    #[test]
    fn a_breakpoint_stops_the_run_and_shows_the_frame() {
        let main = "local function add(a, b)\n  local sum = a + b\n  return sum\nend\nomsi.on(\"start\", function()\n  local t = { n = 1 }\n  print(add(2, 3))\nend)\n";
        let mut st = setup(&[("main.lua", main)], "main.lua");
        st.breakpoints.insert("main.lua".into(), BTreeSet::from([3]));
        let s = Session::start(st, None);
        let out = collect(&s, |o| o.iter().any(|o| matches!(o, Out::Paused { .. })));
        let Some(Out::Paused { file, line, stack, reason }) = out.iter().find(|o| matches!(o, Out::Paused { .. })) else { panic!("{out:?}") };
        assert_eq!((file.as_str(), *line, reason.as_str()), ("main.lua", 3, "breakpoint"));
        assert_eq!(stack[0].func, "add");
        let local = |f: &Frame, n: &str| f.locals.iter().find(|l| l.0 == n).map(|l| l.1.text.clone());
        assert_eq!(local(&stack[0], "sum").as_deref(), Some("5"));
        assert_eq!(stack[1].line, 7);
        let t = stack[1].locals.iter().find(|l| l.0 == "t").unwrap().1.table.unwrap();
        // an expression in the caller's frame, a table opened, then a step out
        s.send(Cmd::Eval("t.n + a".into(), Some(0)));
        s.send(Cmd::Eval("t.n * 10".into(), Some(1)));
        s.send(Cmd::Expand(t));
        let out = collect(&s, |o| o.iter().any(|o| matches!(o, Out::Expanded { .. })));
        let l = lines(&out);
        assert!(l.iter().any(|l| l.ends_with("attempt to perform arithmetic on a nil value (global 't')") || l.contains("nil")), "{l:?}");
        assert!(l.contains(&"> t.n * 10\n10".to_string()), "{l:?}");
        assert!(out.iter().any(|o| matches!(o, Out::Expanded { items, .. } if items[0].0 == "n" && items[0].1.text == "1")));
        s.send(Cmd::StepOut);
        let out = collect(&s, |o| o.iter().any(|o| matches!(o, Out::Paused { .. })));
        // (Lua goes on in the middle of the caller's line: the next line it starts is where a step out stops)
        assert!(out.iter().any(|o| matches!(o, Out::Paused { line: 8, .. })), "{out:?}");
        assert!(lines(&out).contains(&"5".to_string()), "{out:?}");
        s.send(Cmd::Continue);
        let out = collect(&s, |o| o.iter().any(|o| matches!(o, Out::Resumed)));
        assert!(out.iter().any(|o| matches!(o, Out::Resumed)));
    }

    #[test]
    fn an_endless_loop_is_stopped_and_stop_ends_the_run() {
        let s = Session::start(setup(&[("main.lua", "omsi.on(\"start\", function() while true do end end)")], "main.lua"), None);
        s.send(Cmd::Stop);
        let out = collect(&s, |o| o.iter().any(|o| matches!(o, Out::Ended(_))));
        assert!(out.iter().any(|o| matches!(o, Out::Ended(None))), "{out:?}");
    }

    #[test]
    fn a_script_of_objects_drives_its_objects() {
        let main = "omsi.on(\"object_init\", function(id, type, x) omsi.object.set_var(id, \"open\", x) end)\nomsi.on(\"object_click\", function(id) print(\"clicked\", id, omsi.object.var(id, \"open\")) end)\n";
        let s = Session::start(setup(&[("gate.lua", main)], "gate.lua"), None);
        s.send(Cmd::Fire("object_click".into(), vec![Arg::Num(7.0)]));
        let out = collect(&s, |o| lines(o).iter().any(|l| l.starts_with("clicked")) && o.iter().any(|o| matches!(o, Out::State { .. })));
        assert!(lines(&out).contains(&"clicked\t7\t1.0".to_string()), "{:?}", lines(&out));
        assert!(out.iter().any(|o| matches!(o, Out::State { objects, .. } if objects == &vec![(7, vec![("open".to_string(), "1".to_string())])])));
    }

    #[test]
    fn days_are_dated() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(20_773), (2026, 11, 16));
    }
}
