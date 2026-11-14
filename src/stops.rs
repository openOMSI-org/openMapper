//! A bus stop's settings, kept in the strings of its `[busstop]` object as OMSI 2 reads them:
//! the name, the most and the fewest people waiting, the weight of those getting off, the
//! length, the side the platform lies on. The kind of stop is the seventh string (a place the
//! game leaves alone); the game's own say whether a bus stops is the timetable's
//! (`[profile_otherstopping]`, see [`StopKind::serve`]).

use glam::DVec2;

use crate::doc::{tile_origin, Document};
use crate::i18n::tr;

/// Two stops of one name this close are a pair, one each side of the road; the same name
/// farther apart is a clash (a bus is told one stop from the other by the name).
pub const PAIR_REACH: f64 = 40.0;

/// A stop of the map.
#[derive(Debug, Clone)]
pub struct Found {
    pub id: i64,
    pub name: String,
    pub at: DVec2,
}

fn same(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// Every bus stop object of the map.
pub fn all(doc: &Document) -> Vec<Found> {
    doc.tiles
        .iter()
        .flat_map(|(k, t)| t.tile.objects.iter().filter(|o| o.flag == 7 && o.extra.len() >= 4).map(move |o| Found { id: o.id, name: o.extra[0].clone(), at: tile_origin(*k) + DVec2::new(o.pos[0], o.pos[1]) }))
        .collect()
}

/// The stops paired with stop `id`: of its name, within [`PAIR_REACH`].
pub fn pair_of<'a>(all: &'a [Found], id: i64) -> Vec<&'a Found> {
    let Some(me) = all.iter().find(|f| f.id == id) else { return Vec::new() };
    all.iter().filter(|f| f.id != id && same(&f.name, &me.name) && (f.at - me.at).length() < PAIR_REACH).collect()
}

/// The stops with the name of stop `id` that are not its pair: farther than [`PAIR_REACH`].
pub fn clashes<'a>(all: &'a [Found], id: i64) -> Vec<&'a Found> {
    let Some(me) = all.iter().find(|f| f.id == id) else { return Vec::new() };
    all.iter().filter(|f| f.id != id && !me.name.trim().is_empty() && same(&f.name, &me.name) && (f.at - me.at).length() >= PAIR_REACH).collect()
}

/// `wanted`, or the name with a number after it that no stop far from `at` has: a stop of
/// this name beside `at` is its pair and does not count. `id` is the stop itself, if it exists.
pub fn unique_name(all: &[Found], id: Option<i64>, wanted: &str, at: DVec2) -> String {
    let taken = |n: &str| all.iter().any(|f| Some(f.id) != id && same(&f.name, n) && (f.at - at).length() >= PAIR_REACH);
    let wanted = wanted.trim();
    if !taken(wanted) {
        return wanted.to_string();
    }
    // ("Market 2" is asked for again as "Market 3", not "Market 2 2")
    let (base, start) = match wanted.rsplit_once(' ') {
        Some((b, n)) if n.parse::<u32>().is_ok() && !b.is_empty() => (b, n.parse::<u32>().unwrap_or(1) + 1),
        _ => (wanted, 2),
    };
    (start..).map(|n| format!("{base} {n}")).find(|n| !taken(n)).unwrap_or_else(|| wanted.to_string())
}

/// How a bus treats a stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StopKind {
    /// A compulsory stop: every bus stops, people always wait.
    #[default]
    Regular,
    /// Stops on demand: a bus stops only when somebody wants to get on or off.
    OnRequest,
    /// A terminus: the line begins or ends here; everybody gets off.
    Terminus,
}

impl StopKind {
    pub const ALL: [StopKind; 3] = [StopKind::Regular, StopKind::OnRequest, StopKind::Terminus];

    pub fn label(self) -> &'static str {
        match self {
            StopKind::Regular => tr("Compulsory stop", "Обязательная остановка"),
            StopKind::OnRequest => tr("Request stop", "Остановка по требованию"),
            StopKind::Terminus => tr("Terminus", "Конечная остановка"),
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            StopKind::Regular => tr("Every bus stops here and people are always waiting", "Автобус останавливается всегда, пассажиры ждут всегда"),
            StopKind::OnRequest => tr("A bus stops only if somebody waits or presses the stop button", "Автобус останавливается, только если кто-то ждёт или нажал кнопку остановки"),
            StopKind::Terminus => tr("Where a line begins or ends: everybody gets off, a bus waits for its departure", "Где линия начинается или кончается: все выходят, автобус ждёт отправления"),
        }
    }

    fn code(self) -> &'static str {
        match self {
            StopKind::Regular => "",
            StopKind::OnRequest => "request",
            StopKind::Terminus => "terminus",
        }
    }

    fn from_code(s: &str) -> StopKind {
        match s.trim().to_ascii_lowercase().as_str() {
            "request" => StopKind::OnRequest,
            "terminus" => StopKind::Terminus,
            _ => StopKind::Regular,
        }
    }

    /// The timetable's `[profile_otherstopping]` value for a station of this kind:
    /// 1 = served whoever wants it, 0 = served when somebody wants to get on or off.
    pub fn serve(self) -> i32 {
        match self {
            StopKind::OnRequest => 0,
            _ => 1,
        }
    }

    /// What a station's `[profile_otherstopping]` value has the bus do, in OMSI 2's words.
    pub fn of_serve(v: i32) -> Option<&'static str> {
        match v {
            0 => Some(tr("On demand", "По требованию")),
            1 => Some(tr("Always", "Всегда")),
            2 => Some(tr("Not served (checkpoint)", "Не обслуживается (контрольная точка)")),
            3 => Some(tr("Wait for departure time", "Ждать времени отправления")),
            4 => Some(tr("Always + wait for departure time", "Всегда + ждать времени отправления")),
            _ => None,
        }
    }

    /// The `[profile_otherstopping]` values in the order OMSI 2's editor lists them.
    pub const SERVE: [i32; 5] = [1, 3, 2, 0, 4];

    /// What the value does, said in full.
    pub fn serve_hint(v: i32) -> &'static str {
        match v {
            0 => tr("The bus stops only when somebody waits or wants to get off (the game's default)", "Автобус останавливается, только если кто-то ждёт или хочет выйти (так по умолчанию в игре)"),
            1 => tr("The bus always stops here", "Автобус всегда здесь останавливается"),
            2 => tr("The bus drives past without stopping: the stop only marks the way (a checkpoint)", "Автобус проезжает, не останавливаясь: остановка лишь отмечает путь (контрольная точка)"),
            3 => tr("The bus stops only when it is early, and waits there for its departure time", "Автобус останавливается, только если идёт раньше времени, и ждёт там времени отправления"),
            4 => tr("The bus always stops here, and when it is early it waits for its departure time", "Автобус всегда здесь останавливается, а если раньше времени: ждёт времени отправления"),
            _ => "",
        }
    }

    /// The people a new stop of this kind gets: (most waiting, fewest, getting off).
    pub fn defaults(self) -> (i32, i32, i32) {
        match self {
            StopKind::Regular => (10, 2, 10),
            StopKind::OnRequest => (4, 0, 4),
            StopKind::Terminus => (12, 0, 40),
        }
    }
}

/// The side a platform lies on, as the game reads it (the string at index 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Right,
    Left,
    Both,
}

impl Side {
    pub fn label(self) -> &'static str {
        match self {
            Side::Right => tr("right of the road", "справа от дороги"),
            Side::Left => tr("left of the road", "слева от дороги"),
            Side::Both => tr("both sides", "с обеих сторон"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StopSettings {
    pub name: String,
    pub kind: StopKind,
    /// The most and the fewest people waiting when a bus comes.
    pub enter_max: i32,
    pub enter_min: i32,
    /// How many get off here (a weight among the stops ahead: twice the number, twice the riders).
    pub exit: i32,
    /// How far along it people wait and a bus may stand (m).
    pub length: f64,
    pub side: Side,
}

fn num(s: Option<&String>) -> Option<f64> {
    s.map(|s| s.trim().replace(',', ".")).filter(|s| !s.is_empty()).and_then(|s| s.parse::<f64>().ok()).filter(|v| v.is_finite())
}

impl StopSettings {
    /// What a stop's strings say; what they leave out is as the game takes it (one waiting
    /// at most, none at least, getting off the mean of the two, 30 m, the right side).
    pub fn read(strings: &[String]) -> StopSettings {
        let enter_max = num(strings.get(1)).map(|v| v.round() as i32).unwrap_or(1);
        let enter_min = num(strings.get(2)).map(|v| v.round() as i32).unwrap_or(0);
        let exit = num(strings.get(3)).map(|v| v.round() as i32).unwrap_or((enter_max + enter_min) / 2).max(0);
        StopSettings {
            name: strings.first().cloned().unwrap_or_default(),
            kind: strings.get(6).map(|s| StopKind::from_code(s)).unwrap_or_default(),
            enter_max,
            enter_min,
            exit,
            length: num(strings.get(4)).unwrap_or(30.0),
            side: match num(strings.get(5)).unwrap_or(0.0).round() as i32 {
                1 => Side::Left,
                2 => Side::Both,
                _ => Side::Right,
            },
        }
    }

    /// The strings for these settings, the stop's other strings kept; trailing empty ones stay
    /// (a stop has seven).
    pub fn write(&self, strings: &mut Vec<String>) {
        while strings.len() < 7 {
            strings.push(String::new());
        }
        strings[0] = self.name.clone();
        strings[1] = self.enter_max.max(0).to_string();
        strings[2] = self.enter_min.clamp(0, self.enter_max.max(0)).to_string();
        strings[3] = self.exit.max(0).to_string();
        // (30 m is what the game takes for an empty string)
        strings[4] = if (self.length - 30.0).abs() < 1e-6 { String::new() } else { format!("{}", (self.length * 10.0).round() / 10.0) };
        strings[5] = match self.side {
            Side::Right => String::new(),
            Side::Left => "1".into(),
            Side::Both => "2".into(),
        };
        strings[6] = self.kind.code().to_string();
    }

    /// A new stop of `kind` called `name`.
    pub fn new(name: &str, kind: StopKind) -> StopSettings {
        let (enter_max, enter_min, exit) = kind.defaults();
        StopSettings { name: name.to_string(), kind, enter_max, enter_min, exit, length: 30.0, side: Side::Right }
    }

    /// The strings of a new stop of `kind`.
    pub fn new_strings(name: &str, kind: StopKind) -> Vec<String> {
        let mut v = Vec::new();
        StopSettings::new(name, kind).write(&mut v);
        v
    }

    /// Change the kind and give the people the kind's usual numbers.
    pub fn set_kind(&mut self, kind: StopKind) {
        let (enter_max, enter_min, exit) = kind.defaults();
        self.kind = kind;
        self.enter_max = enter_max;
        self.enter_min = enter_min;
        self.exit = exit;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(id: i64, name: &str, x: f64) -> Found {
        Found { id, name: name.into(), at: DVec2::new(x, 0.0) }
    }

    #[test]
    fn a_pair_is_near_and_a_clash_is_far() {
        let all = vec![at(1, "Market", 0.0), at(2, "market ", 12.0), at(3, "Market", 500.0), at(4, "Station", 0.0)];
        assert_eq!(pair_of(&all, 1).iter().map(|f| f.id).collect::<Vec<_>>(), vec![2]);
        assert_eq!(clashes(&all, 1).iter().map(|f| f.id).collect::<Vec<_>>(), vec![3]);
        assert!(clashes(&all, 4).is_empty());
    }

    #[test]
    fn a_name_is_made_unique_but_a_pair_keeps_it() {
        let all = vec![at(1, "Market", 0.0), at(2, "Market 2", 300.0)];
        // beside the first: its pair, the name stays
        assert_eq!(unique_name(&all, None, "Market", DVec2::new(10.0, 0.0)), "Market");
        // far from it: a second Market
        assert_eq!(unique_name(&all, None, "Market", DVec2::new(900.0, 0.0)), "Market 3");
        assert_eq!(unique_name(&all, None, "Market 2", DVec2::new(900.0, 0.0)), "Market 3");
        // a stop is not its own clash
        assert_eq!(unique_name(&all, Some(1), "Market", DVec2::new(0.0, 0.0)), "Market");
    }

    #[test]
    fn a_stock_stop_reads_as_the_game_does() {
        let s = StopSettings::read(&["Krankenhaus".into(), "15".into(), "0".into(), "0".into()]);
        assert_eq!((s.enter_max, s.enter_min, s.exit, s.length, s.side, s.kind), (15, 0, 0, 30.0, Side::Right, StopKind::Regular));
        // no exit string: the mean of the two
        let s = StopSettings::read(&["A".into(), "10".into(), "4".into()]);
        assert_eq!(s.exit, 7);
    }

    #[test]
    fn settings_round_trip_through_the_strings() {
        let mut s = StopSettings::new("Markt", StopKind::Terminus);
        s.length = 45.5;
        s.side = Side::Left;
        let mut v = Vec::new();
        s.write(&mut v);
        assert_eq!(v.len(), 7);
        assert_eq!(StopSettings::read(&v), s);
        // the fewest never exceeds the most
        s.enter_max = 3;
        s.enter_min = 9;
        s.write(&mut v);
        assert_eq!(StopSettings::read(&v).enter_min, 3);
    }
}
