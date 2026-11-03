//! The map's timetable (`TTData`): bus stops, station links, trips, tracks and lines, read
//! with openOMSI's own `omsi-timetable` and written back in OMSI 2's formats.
//!
//! Only what was changed is written: a stock map's files stay as they are until a line,
//! trip or tour of it is edited.

pub mod caruse;
pub mod network;
pub mod route;

use std::collections::BTreeSet;
use std::path::Path;

use omsi_timetable::{BusStopEntry, Line, StnLink, StnLinkEntry, TimetableData, Track, TrackEntry, Trip};

use crate::doc::safe::Op;
use crate::doc::{Document, TileKey};

/// Days a tour runs on: bits 0-6 Monday…Sunday, 7 public holidays, 8 school holidays,
/// 9 school days. 1023: every day.
pub const ALL_DAYS: i32 = 1023;

#[derive(Debug, Clone, Default)]
pub struct Timetable {
    pub stops: Vec<BusStopEntry>,
    pub links: Vec<StnLink>,
    pub trips: Vec<Trip>,
    pub tracks: Vec<Track>,
    pub lines: Vec<Line>,
    /// The tiles in the order the files' tile indices count them (global.cfg's `[map]` list
    /// when they were read or last written).
    pub order: Vec<TileKey>,
    /// Changed since read or saved.
    pub dirty: bool,
    /// Counts each change, so a save that is still writing can tell a later edit from the one it copied.
    pub edit: u64,
    /// Files to write (lower-case names) and to take away on the next save.
    changed: BTreeSet<String>,
    removed: BTreeSet<String>,
    /// The stop list and the station links are written again.
    stops_changed: bool,
    links_changed: bool,
    /// Trips whose route is to be found again (their stops changed).
    pub reroute: BTreeSet<String>,
    /// Public holidays and school holidays (`Holidays.txt`): which tours run on a day.
    pub calendar: omsi_map::Calendar,
    calendar_changed: bool,
    /// Which vehicles drive the lines (`car_use/*.ocu`), and the files to write and take away.
    pub car_use: Vec<caruse::CarUseFile>,
    car_changed: BTreeSet<String>,
    car_removed: BTreeSet<String>,
}

fn key(name: &str) -> String {
    name.to_lowercase()
}

impl Timetable {
    /// Read `TTData` of the map folder `dir`; `order` is global.cfg's tile list.
    pub fn load(dir: &Path, order: &[TileKey]) -> Timetable {
        let d = TimetableData::load(dir);
        for e in &d.errors {
            log::warn!("timetable: {e}");
        }
        let n = (d.lines.len(), d.trips.len(), d.tracks.len(), d.bus_stops.len());
        if n != (0, 0, 0, 0) {
            log::info!("timetable: {} lines, {} trips, {} tracks, {} stops", n.0, n.1, n.2, n.3);
        }
        let mut calendar = omsi_map::Calendar::load(&omsi_cfg::resolve_path(dir, "Holidays.txt")).unwrap_or_default();
        // (in the order of the days, as the Lines window lists them)
        calendar.holidays.sort_by_key(|h| h.date);
        calendar.ranges.sort_by_key(|r| r.start);
        Timetable { stops: d.bus_stops, links: d.stn_links, trips: d.trips, tracks: d.tracks, lines: d.lines, order: order.to_vec(), calendar, car_use: caruse::load_dir(dir), ..Default::default() }
    }

    pub fn trip(&self, name: &str) -> Option<&Trip> {
        self.trips.iter().find(|t| t.name.eq_ignore_ascii_case(name))
    }

    pub fn trip_mut(&mut self, name: &str) -> Option<&mut Trip> {
        self.trips.iter_mut().find(|t| t.name.eq_ignore_ascii_case(name))
    }

    pub fn track(&self, name: &str) -> Option<&Track> {
        self.tracks.iter().find(|t| track_name(t).eq_ignore_ascii_case(name))
    }

    /// The name stop object `id` has in the timetable.
    pub fn stop_name(&self, id: i64) -> Option<&str> {
        self.stops.iter().find(|s| s.object_id == id).map(|s| s.name.trim())
    }

    pub fn touch_line(&mut self, name: &str) {
        self.changed.insert(key(&format!("{name}.ttl")));
        self.removed.remove(&key(&format!("{name}.ttl")));
        self.mark();
    }

    pub fn touch_trip(&mut self, name: &str) {
        self.changed.insert(key(&format!("{name}.ttp")));
        self.removed.remove(&key(&format!("{name}.ttp")));
        self.mark();
    }

    pub fn touch_stops(&mut self) {
        self.stops_changed = true;
        self.mark();
    }

    /// A trip's stops changed: its route is found again.
    pub fn touch_route(&mut self, name: &str) {
        self.touch_trip(name);
        self.reroute.insert(key(name));
    }

    /// Put back as it was (undo, redo) in place of `replaced`: every file of it is written
    /// again on the next save, and those only `replaced` had are taken away.
    pub fn restored(&mut self, replaced: &Timetable) {
        let names = |t: &Timetable| -> BTreeSet<String> {
            t.lines.iter().map(|l| key(&format!("{}.ttl", l.name))).chain(t.trips.iter().map(|x| key(&format!("{}.ttp", x.name)))).chain(t.tracks.iter().map(|x| key(&format!("{}.ttr", track_name(x))))).collect()
        };
        let mine = names(self);
        self.removed = names(replaced).difference(&mine).cloned().chain(replaced.removed.iter().cloned()).filter(|n| !mine.contains(n)).collect();
        self.changed = mine;
        self.stops_changed = true;
        self.links_changed = true;
        self.calendar_changed = true;
        let have: BTreeSet<String> = self.car_use.iter().map(|c| c.file.to_lowercase()).collect();
        self.car_removed = replaced.car_use.iter().map(|c| c.file.to_lowercase()).filter(|f| !have.contains(f)).chain(replaced.car_removed.iter().cloned()).filter(|f| !have.contains(f)).collect();
        self.car_changed = have;
        self.reroute.clear();
        self.mark();
    }

    pub fn touch_calendar(&mut self) {
        self.calendar_changed = true;
        self.mark();
    }

    /// The same lines, trips, tours and stops (what the Lines window edits; not the routes
    /// found for them or what is still to be saved).
    pub fn same_content(&self, o: &Timetable) -> bool {
        self.lines == o.lines && self.trips == o.trips && self.stops == o.stops && self.calendar == o.calendar && self.car_use == o.car_use
    }

    /// A new line (its name made unique).
    pub fn add_line(&mut self, name: &str) -> String {
        let name = self.unique(name, |t, n| t.lines.iter().any(|l| l.name.eq_ignore_ascii_case(n)));
        self.lines.push(Line { name: name.clone(), user_allowed: true, priority: 1, ..Default::default() });
        self.touch_line(&name);
        name
    }

    pub fn rename_line(&mut self, old: &str, new: &str) -> Option<String> {
        let new = new.trim();
        if new.is_empty() || !valid_file_name(new) {
            return None;
        }
        if !old.eq_ignore_ascii_case(new) && self.lines.iter().any(|l| l.name.eq_ignore_ascii_case(new)) {
            return None;
        }
        let l = self.lines.iter_mut().find(|l| l.name == old)?;
        l.name = new.to_string();
        self.removed.insert(key(&format!("{old}.ttl")));
        self.changed.remove(&key(&format!("{old}.ttl")));
        self.touch_line(new);
        self.car_use_follow(old, Some(new));
        // the trips that show it as their line number
        let trips: Vec<String> = self.trips.iter().filter(|t| t.line.trim() == old).map(|t| t.name.clone()).collect();
        for t in trips {
            if let Some(t) = self.trip_mut(&t) {
                t.line = new.to_string();
            }
            self.touch_trip(&t);
        }
        Some(new.to_string())
    }

    /// A copy of a line under a new name (a variant to change): its trips copied with its
    /// name in theirs ("76_A-B" → "76 copy_A-B"), their ways along, its tours driving the
    /// copies. The copy's name.
    pub fn duplicate_line(&mut self, name: &str) -> Option<String> {
        let src = self.lines.iter().find(|l| l.name == name)?.clone();
        let new = self.add_line(&format!("{name} {}", crate::i18n::tr("copy", "копия")));
        let mut trips: BTreeSet<String> = src.tours.iter().flat_map(|t| t.trips.iter().map(|x| x.trip.clone())).collect();
        trips.extend(self.trips.iter().filter(|t| t.line.trim() == name).map(|t| t.name.clone()));
        let mut renamed: std::collections::HashMap<String, String> = Default::default();
        for t in trips {
            let Some(old) = self.trip(&t).cloned() else { continue };
            let base = match t.strip_prefix(&format!("{name}_")) {
                Some(rest) => format!("{new}_{rest}"),
                None => format!("{new}_{t}"),
            };
            let copy = self.unique(&base, |tt, n| tt.trips.iter().any(|x| x.name.eq_ignore_ascii_case(n)));
            let line = if old.line.trim() == name { new.clone() } else { old.line.clone() };
            self.trips.push(Trip { name: copy.clone(), line, path: Default::default(), ..old });
            self.touch_trip(&copy);
            if let Some(track) = self.track(&t).map(|x| x.entries.clone()) {
                self.tracks.push(Track { path: format!("{copy}.ttr").into(), entries: track });
                self.changed.insert(key(&format!("{copy}.ttr")));
            } else {
                self.reroute.insert(key(&copy));
            }
            renamed.insert(t.to_lowercase(), copy);
        }
        let l = self.lines.iter_mut().find(|l| l.name == new)?;
        l.user_allowed = src.user_allowed;
        l.priority = src.priority;
        l.tours = src.tours;
        for x in l.tours.iter_mut().flat_map(|t| t.trips.iter_mut()) {
            if let Some(n) = renamed.get(&x.trip.to_lowercase()) {
                x.trip = n.clone();
            }
        }
        self.touch_line(&new);
        Some(new)
    }

    /// Take a line away, and the trips no other line runs.
    pub fn delete_line(&mut self, name: &str) {
        let Some(i) = self.lines.iter().position(|l| l.name == name) else { return };
        let l = self.lines.remove(i);
        self.car_use_follow(name, None);
        self.removed.insert(key(&format!("{name}.ttl")));
        self.changed.remove(&key(&format!("{name}.ttl")));
        let mut trips: BTreeSet<String> = l.tours.iter().flat_map(|t| t.trips.iter().map(|x| x.trip.clone())).collect();
        trips.extend(self.trips.iter().filter(|t| t.line.trim() == name).map(|t| t.name.clone()));
        for t in trips {
            if !self.lines.iter().any(|l| l.tours.iter().any(|to| to.trips.iter().any(|x| x.trip.eq_ignore_ascii_case(&t)))) {
                self.delete_trip(&t);
            }
        }
        self.mark();
    }

    /// A new trip of a line, its stops `stops`.
    pub fn add_trip(&mut self, line: &str, name: &str, stops: Vec<i64>) -> String {
        let name = self.unique(name, |t, n| t.trips.iter().any(|x| x.name.eq_ignore_ascii_case(n)));
        let terminus = stops.last().and_then(|id| self.stop_name(*id)).unwrap_or_default().to_string();
        self.trips.push(Trip {
            name: name.clone(),
            terminus,
            line: line.to_string(),
            stations: stops,
            profiles: vec![omsi_timetable::TripProfile { name: "standard".into(), factor: 10.0, ..Default::default() }],
            ..Default::default()
        });
        self.touch_route(&name);
        name
    }

    pub fn rename_trip(&mut self, old: &str, new: &str) -> Option<String> {
        let new = new.trim();
        if new.is_empty() || !valid_file_name(new) {
            return None;
        }
        if !old.eq_ignore_ascii_case(new) && self.trip(new).is_some() {
            return None;
        }
        self.trip_mut(old)?.name = new.to_string();
        for ext in ["ttp", "ttr"] {
            self.removed.insert(key(&format!("{old}.{ext}")));
            self.changed.remove(&key(&format!("{old}.{ext}")));
        }
        if let Some(t) = self.tracks.iter_mut().find(|t| track_name(t).eq_ignore_ascii_case(old)) {
            t.path = format!("{new}.ttr").into();
            self.changed.insert(key(&format!("{new}.ttr")));
        }
        for l in &mut self.lines {
            let mut hit = false;
            for t in l.tours.iter_mut().flat_map(|t| t.trips.iter_mut()) {
                if t.trip.eq_ignore_ascii_case(old) {
                    t.trip = new.to_string();
                    hit = true;
                }
            }
            if hit {
                self.changed.insert(key(&format!("{}.ttl", l.name)));
            }
        }
        self.touch_trip(new);
        Some(new.to_string())
    }

    /// Take a trip away, its track and the departures of it.
    pub fn delete_trip(&mut self, name: &str) {
        self.trips.retain(|t| !t.name.eq_ignore_ascii_case(name));
        self.tracks.retain(|t| !track_name(t).eq_ignore_ascii_case(name));
        for ext in ["ttp", "ttr"] {
            self.removed.insert(key(&format!("{name}.{ext}")));
            self.changed.remove(&key(&format!("{name}.{ext}")));
        }
        let mut touched = Vec::new();
        for l in &mut self.lines {
            let before: usize = l.tours.iter().map(|t| t.trips.len()).sum();
            for t in &mut l.tours {
                t.trips.retain(|x| !x.trip.eq_ignore_ascii_case(name));
            }
            if l.tours.iter().map(|t| t.trips.len()).sum::<usize>() != before {
                touched.push(l.name.clone());
            }
        }
        for l in touched {
            self.touch_line(&l);
        }
        self.reroute.remove(&key(name));
        self.mark();
    }

    /// The route found for a trip: its track (`<trip>.ttr`) and the station links between
    /// its stops (`StnLinks.cfg`, which OMSI 2 drives a bus trip by).
    pub fn set_route(&mut self, trip: &str, track: Vec<TrackEntry>, links: Vec<StnLink>) {
        match self.tracks.iter_mut().find(|t| track_name(t).eq_ignore_ascii_case(trip)) {
            Some(t) => t.entries = track,
            None => self.tracks.push(Track { path: format!("{trip}.ttr").into(), entries: track }),
        }
        self.changed.insert(key(&format!("{trip}.ttr")));
        self.removed.remove(&key(&format!("{trip}.ttr")));
        for l in links {
            self.links.retain(|x| !(x.from_id == l.from_id && x.to_id == l.to_id));
            self.links.push(l);
        }
        self.links_changed = true;
        self.mark();
    }

    /// The stop list brought in line with the map's stop objects (`stop_entries`): names and
    /// tiles follow the objects, new stops are added. True when anything changed.
    pub fn sync_stops(&mut self, entries: Vec<BusStopEntry>) -> bool {
        let mut changed = false;
        for e in entries {
            match self.stops.iter_mut().find(|s| s.object_id == e.object_id) {
                Some(s) => {
                    if s.name.trim() != e.name || s.group != e.group || s.params[0] != e.params[0] {
                        s.name = e.name;
                        s.group = e.group;
                        s.params[0] = e.params[0];
                        changed = true;
                    }
                }
                None => {
                    self.stops.push(e);
                    changed = true;
                }
            }
        }
        if changed {
            self.touch_stops();
        }
        changed
    }

    /// `base`, or `base 2`, `base 3`… when `taken` says that is used.
    fn unique(&self, base: &str, taken: impl Fn(&Timetable, &str) -> bool) -> String {
        let base = base.trim();
        let base = if base.is_empty() { "1" } else { base };
        if !taken(self, base) {
            return base.to_string();
        }
        (2..).map(|i| format!("{base} {i}")).find(|n| !taken(self, n)).unwrap()
    }

    /// The tile indices made to count in `order` (tiles were added or taken away since
    /// the files were read): every file that holds one is written again.
    pub(crate) fn renumber_tiles(&mut self, order: &[TileKey]) {
        if self.order.is_empty() || self.order == order {
            self.order = order.to_vec();
            return;
        }
        let map = |i: f64| renumbered(&self.order, i, order);
        for s in &mut self.stops {
            s.group = map(s.group as f64) as i32;
        }
        for l in &mut self.links {
            for e in &mut l.entries {
                e.values[2] = map(e.values[2]);
            }
        }
        let names: Vec<String> = self.tracks.iter().map(track_name).collect();
        for t in &mut self.tracks {
            for e in &mut t.entries {
                if e.values.len() > 2 {
                    e.values[2] = map(e.values[2]);
                }
            }
        }
        for n in names {
            self.changed.insert(key(&format!("{n}.ttr")));
        }
        self.stops_changed = true;
        self.links_changed = true;
        self.order = order.to_vec();
    }
}

/// Tile index `i` of files written with the tile list `old`, counted in `order`.
fn renumbered(old: &[TileKey], i: f64, order: &[TileKey]) -> f64 {
    old.get(i as usize).and_then(|k| order.iter().position(|o| o == k)).map(|n| n as f64).unwrap_or(i)
}

/// The stops' tiles as their objects stand now (one moved over into another tile names
/// its new one), written on the next save.
pub fn follow_stop_tiles(doc: &mut Document) {
    if doc.timetable.stops.is_empty() {
        return;
    }
    let mut at: std::collections::HashMap<i64, i32> = std::collections::HashMap::new();
    for (i, k) in doc.order.iter().enumerate() {
        if let Some(t) = doc.tiles.get(k) {
            at.extend(t.tile.objects.iter().map(|o| (o.id, i as i32)));
        }
    }
    let tt = &mut doc.timetable;
    let moved = tt.stops.iter().any(|s| at.get(&s.object_id).is_some_and(|g| *g as f64 != renumbered(&tt.order, s.group as f64, &doc.order)));
    if !moved {
        return;
    }
    // (the indices counted in today's list first, then the moved ones set)
    tt.renumber_tiles(&doc.order);
    for s in &mut tt.stops {
        if let Some(g) = at.get(&s.object_id) {
            s.group = *g;
        }
    }
    tt.touch_stops();
}

/// The `Busstops.cfg` entries of stop objects (tile, IDCode): the name is the object's
/// first label, the number after the id its fourth (as OMSI's editor writes them).
pub fn stop_entries(doc: &Document, ids: &[(TileKey, i64)]) -> Vec<BusStopEntry> {
    ids.iter()
        .filter_map(|&(k, id)| {
            let o = doc.object(k, id)?;
            let tile = doc.order.iter().position(|t| *t == k)?;
            let name = o.extra.first().map(|s| s.trim().to_string()).unwrap_or_default();
            let offset = o.extra.get(3).and_then(|s| s.trim().parse::<f64>().ok()).unwrap_or(0.0);
            Some(BusStopEntry { name, group: tile as i32, object_id: id, params: [offset, 0.0, 0.0] })
        })
        .collect()
}

/// Where stop object `id` stands (world).
pub fn stop_world(doc: &Document, files: &mut network::Files, id: i64) -> Option<glam::DVec3> {
    let (k, o) = doc.tiles.iter().find_map(|(k, t)| t.tile.objects.iter().find(|o| o.id == id).map(|o| (*k, o)))?;
    let abs = files.get(&o.file).map(|f| f.abs_height).unwrap_or(false);
    Some(doc.object_world(k, o, abs))
}

/// A name a file can have on every system.
pub fn valid_file_name(n: &str) -> bool {
    !n.chars().any(|c| matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control()) && n.trim() == n && !n.ends_with('.')
}

pub fn track_name(t: &Track) -> String {
    t.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// What saving the timetable writes and takes away in the map folder `map_dir` (`order`:
/// the tile list as saved): only what changed. The map's save carries it out with its own
/// files, all or nothing; `saved` then marks it done.
pub fn plan(tt: &mut Timetable, map_dir: &Path, order: &[TileKey]) -> Vec<Op> {
    let mut ops = Vec::new();
    // (a tile taken away renumbers the files' tile indices even when nothing of the
    // timetable was edited)
    let held = !(tt.stops.is_empty() && tt.links.is_empty() && tt.tracks.is_empty());
    if !tt.dirty && !(held && !tt.order.is_empty() && tt.order != order) {
        tt.order = order.to_vec();
        return ops;
    }
    tt.renumber_tiles(order);
    let dir = map_dir.join("TTData");
    for f in &tt.removed {
        if let Some(p) = find_file(&dir, f) {
            ops.push(Op::Remove(p));
        }
    }
    // (a file there in another case is written under its own name: on a disk that ignores
    // case, writing the new name and removing the old would remove both)
    let at = |name: &str| find_file(&dir, &name.to_lowercase()).unwrap_or_else(|| dir.join(name));
    for f in &tt.changed {
        let (stem, ext) = f.rsplit_once('.').unwrap_or((f.as_str(), ""));
        let text = match ext {
            "ttl" => tt.lines.iter().find(|l| key(&l.name) == stem).map(|l| (l.name.clone(), line_text(l))),
            "ttp" => tt.trips.iter().find(|t| key(&t.name) == stem).map(|t| (t.name.clone(), trip_text(t))),
            "ttr" => tt.tracks.iter().find(|t| key(&track_name(t)) == stem).map(|t| (track_name(t), track_text(t))),
            _ => None,
        };
        if let Some((name, text)) = text {
            ops.push(Op::Write(at(&format!("{name}.{ext}")), encode(&text)));
        }
    }
    if tt.stops_changed || !dir.join("Busstops.cfg").is_file() {
        ops.push(Op::Write(at("Busstops.cfg"), encode(&busstops_text(&tt.stops))));
    }
    if tt.links_changed {
        ops.push(Op::Write(at("StnLinks.cfg"), encode(&stnlinks_text(&tt.links, &tt.stops))));
    }
    if tt.calendar_changed {
        let p = find_file(map_dir, "holidays.txt").unwrap_or_else(|| map_dir.join("Holidays.txt"));
        ops.push(Op::Write(p, encode(&holidays_text(&tt.calendar))));
    }
    ops.extend(caruse::plan(tt, map_dir));
    ops
}

impl Timetable {
    pub(crate) fn mark(&mut self) {
        self.dirty = true;
        self.edit = self.edit.wrapping_add(1);
    }
}

/// The planned files were written.
pub fn saved(tt: &mut Timetable) {
    tt.removed.clear();
    tt.changed.clear();
    tt.stops_changed = false;
    tt.links_changed = false;
    tt.calendar_changed = false;
    caruse::saved(tt);
    tt.dirty = false;
}

/// Write what changed into `map_dir/TTData` by itself (all or nothing).
#[cfg(test)]
pub fn save_to(tt: &mut Timetable, map_dir: &Path, order: &[TileKey]) -> anyhow::Result<()> {
    let ops = plan(tt, map_dir, order);
    crate::doc::safe::commit(&ops)?;
    saved(tt);
    Ok(())
}

/// A file of the folder by its lower-case name.
fn find_file(dir: &Path, lower: &str) -> Option<std::path::PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase() == lower).unwrap_or(false))
}

/// The text as OMSI 2 writes TTData: Windows-1252 with CR LF; text that code page cannot
/// hold (Cyrillic stop names) as UTF-16 with a byte order mark, which openOMSI reads too.
pub fn encode(text: &str) -> Vec<u8> {
    let crlf = text.replace("\r\n", "\n").replace('\n', "\r\n");
    let (bytes, _, lossy) = omsi_cfg::codepage::CodePage::Windows1252.encoding().encode(&crlf);
    if !lossy {
        return bytes.into_owned();
    }
    let mut out = vec![0xFF, 0xFE];
    for u in crlf.encode_utf16() {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out
}

fn header(kind: &str) -> String {
    let rule = "-".repeat(kind.len() + 2);
    format!("{rule}\n{kind}\n{rule}\n\nCreated with openMapper {}\n\n", env!("CARGO_PKG_VERSION"))
}

/// A number as OMSI writes the fractional ones in TTData (ten decimals).
fn f10(v: f64) -> String {
    format!("{v:.10}")
}

/// An id or index kept whole.
fn int(v: f64) -> String {
    format!("{}", v.round() as i64)
}

pub fn busstops_text(stops: &[BusStopEntry]) -> String {
    let mut o = header("Time Table BusStopList File");
    for s in stops {
        o.push_str(&format!("[busstop]\n{}\n{}\n{}\n{}\n{}\n{}\n\n\n", s.name.trim(), s.group, s.object_id, f10(s.params[0]), int(s.params[1]), int(s.params[2])));
    }
    o
}

pub fn stnlinks_text(links: &[StnLink], stops: &[BusStopEntry]) -> String {
    let name = |id: i64| stops.iter().find(|s| s.object_id == id).map(|s| s.name.trim().to_string()).unwrap_or_else(|| id.to_string());
    let mut o = header("Time Table StnLinkList File");
    for l in links {
        o.push_str(&format!("{} ==> {}\n\n[StnLink]\n{}\n{}\n{}\n", name(l.from_id), name(l.to_id), f10(l.length), l.from_id, l.to_id));
        for (i, p) in l.params.iter().enumerate() {
            // (the last two: a flag and the index of the last entry)
            o.push_str(&if i < 4 { f10(*p) } else { int(*p) });
            o.push('\n');
        }
        o.push('\n');
        for (i, e) in l.entries.iter().enumerate() {
            o.push_str(&stnlink_entry_text(i, e));
        }
    }
    o
}

fn stnlink_entry_text(i: usize, e: &StnLinkEntry) -> String {
    let v = &e.values;
    format!("   {i}:\n[StnLink_entry]\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n\n", int(v[0]), int(v[1]), int(v[2]), f10(v[3]), int(v[4]), int(v[5]), int(v[6]))
}

pub fn trip_text(t: &Trip) -> String {
    let mut o = header("Time Table Trip File");
    o.push_str(&format!("[trip]\n{}\n{}\n{}\n\n", t.display_name, t.terminus, t.line));
    if t.train_reverse {
        o.push_str("[trainreverse]\n\n");
    }
    o.push_str(".........................\n        Stations\n.........................\n\n");
    for s in &t.stations {
        o.push_str(&format!("[station_typ2]\n{s}\n\n"));
    }
    for s in &t.stations_legacy {
        o.push_str("[station]\n");
        for l in s {
            o.push_str(l);
            o.push('\n');
        }
        o.push('\n');
    }
    o.push_str("\n.........................\n        Profiles\n.........................\n\n");
    for p in &t.profiles {
        o.push_str(&format!("[profile]\n{}\n{:.3}\n\n", p.name, p.factor));
        for (i, v) in &p.man_arr_time {
            o.push_str(&format!("[profile_man_arr_time]\n{i}\n{v:.3}\n"));
        }
        for (i, v) in &p.man_dep_time {
            o.push_str(&format!("[profile_man_dep_time]\n{i}\n{v:.3}\n"));
        }
        for (i, v) in &p.other_stopping {
            o.push_str(&format!("[profile_otherstopping]\n{i}\n{v}\n"));
        }
        o.push('\n');
    }
    o
}

pub fn track_text(t: &Track) -> String {
    let mut o = header("Time Table Track File");
    for (i, e) in t.entries.iter().enumerate() {
        o.push_str(&format!("   {i}:\n[track_entry]\n"));
        for (k, v) in e.values.iter().enumerate() {
            // id, path, tile, internal number: whole; then the length and the rest
            o.push_str(&if k == 4 { f10(*v) } else if v.fract() == 0.0 { int(*v) } else { f10(*v) });
            o.push('\n');
        }
        o.push('\n');
    }
    o
}

/// `Holidays.txt` as OMSI 2 has it: per year its school holidays (`[holidays]` first day,
/// last day, name), then its public holidays (`[holiday]` day, name).
pub fn holidays_text(c: &omsi_map::Calendar) -> String {
    let mut years: BTreeSet<i32> = c.ranges.iter().map(|r| r.start / 10_000).chain(c.holidays.iter().map(|h| h.date / 10_000)).collect();
    if years.is_empty() {
        years.insert(0);
    }
    let mut o = String::new();
    for y in years {
        o.push_str(&format!("-----------------{y}------------------------------\n\n"));
        let mut ranges: Vec<_> = c.ranges.iter().filter(|r| r.start / 10_000 == y).collect();
        ranges.sort_by_key(|r| r.start);
        for r in ranges {
            o.push_str(&format!("[holidays]\n{}\n{}\n{}\n\n", r.start, r.end, r.name.trim()));
        }
        o.push_str("..............................\n\n");
        let mut days: Vec<_> = c.holidays.iter().filter(|h| h.date / 10_000 == y).collect();
        days.sort_by_key(|h| h.date);
        for h in days {
            o.push_str(&format!("[holiday]\n{}\n{}\n\n", h.date, h.name.trim()));
        }
    }
    o
}

pub fn line_text(l: &Line) -> String {
    let mut o = header("Time Table Line File");
    if l.user_allowed {
        o.push_str("[userallowed]\n\n");
    }
    o.push_str(&format!("[priority]\n{}\n", l.priority));
    for t in &l.tours {
        o.push_str("------------------------------------\n\n");
        o.push_str(&format!("[newtour]\n{}\n{}\n{}\n\n------------------------------------\n\n", t.number, t.ai_group, t.extra));
        for tr in &t.trips {
            let secs = (tr.departure as f64 * 60.0).round() as i64;
            o.push_str(&format!("  Dep.: {}:{}:{}\n", secs / 3600, secs / 60 % 60, secs % 60));
            o.push_str(&format!("[addtrip]\n{}\n{}\n{:.3}\n\n", tr.trip, tr.profile, tr.departure));
        }
    }
    o
}

/// When a trip is at each of its stops (minutes after it leaves): (arrival, departure, the
/// profile fixes it), as openOMSI reckons them: the first stop at 0, the last at the
/// profile's time, the fixed ones as given and the others spread between them by the
/// distance along the route (`legs`: metres from each stop to the next; a missing leg
/// counts 500 m, as there).
pub fn stop_times(n: usize, legs: &[f64], p: &omsi_timetable::TripProfile) -> Vec<(f32, f32, bool)> {
    let duration = if p.factor > 0.0 { p.factor } else { 10.0 };
    let mut arr: Vec<Option<f32>> = vec![None; n];
    let mut dep: Vec<Option<f32>> = vec![None; n];
    for &(i, m) in &p.man_arr_time {
        if let Some(a) = usize::try_from(i).ok().and_then(|i| arr.get_mut(i)) {
            *a = Some(m);
        }
    }
    for &(i, m) in &p.man_dep_time {
        if let Some(d) = usize::try_from(i).ok().and_then(|i| dep.get_mut(i)) {
            *d = Some(m);
        }
    }
    let fixed: Vec<bool> = (0..n).map(|i| arr[i].is_some() || dep[i].is_some()).collect();
    if n > 0 && !fixed[0] {
        dep[0] = Some(0.0);
    }
    if n > 1 && !fixed[n - 1] {
        arr[n - 1] = Some(duration);
    }
    let mut along = vec![0.0f64; n];
    for i in 1..n {
        along[i] = along[i - 1] + legs.get(i - 1).copied().filter(|l| *l > 0.0).unwrap_or(500.0).max(1.0);
    }
    let timed: Vec<usize> = (0..n).filter(|&i| arr[i].is_some() || dep[i].is_some()).collect();
    let mut out = Vec::with_capacity(n);
    let mut last = 0.0f32;
    for i in 0..n {
        let (a, d) = match (arr[i], dep[i]) {
            (Some(a), Some(d)) => (a, d.max(a)),
            (Some(a), None) => (a, a),
            (None, Some(d)) => (d, d),
            (None, None) => {
                let p = timed.iter().rev().find(|&&k| k < i).copied();
                let q = timed.iter().find(|&&k| k > i).copied();
                let t = match (p, q) {
                    (Some(p), Some(q)) => {
                        let t0 = dep[p].or(arr[p]).unwrap_or(0.0);
                        let t1 = arr[q].or(dep[q]).unwrap_or(t0);
                        let span = along[q] - along[p];
                        if span > 0.0 { t0 + (t1 - t0) * ((along[i] - along[p]) / span) as f32 } else { t0 }
                    }
                    (Some(p), None) => dep[p].or(arr[p]).unwrap_or(0.0),
                    (None, Some(q)) => arr[q].or(dep[q]).unwrap_or(0.0),
                    (None, None) => 0.0,
                };
                (t, t)
            }
        };
        let a = a.max(last);
        let d = d.max(a);
        last = d;
        out.push((a, d, fixed[i]));
    }
    out
}

/// Fix stop `i`'s time in a trip's profile (minutes after it leaves; the last stop its
/// arrival, the others their departure), or let it follow the distance again (None).
pub fn set_stop_time(p: &mut omsi_timetable::TripProfile, n: usize, i: usize, m: Option<f32>) {
    let k = i as i32;
    p.man_arr_time.retain(|x| x.0 != k);
    p.man_dep_time.retain(|x| x.0 != k);
    if let Some(m) = m {
        if i + 1 == n {
            p.man_arr_time.push((k, m));
            p.man_arr_time.sort_by_key(|x| x.0);
        } else {
            p.man_dep_time.push((k, m));
            p.man_dep_time.sort_by_key(|x| x.0);
        }
    }
}

/// Minutes after midnight as "h:mm" (past midnight: 24:10 …).
pub fn clock(minutes: f32) -> String {
    let m = minutes.round().max(0.0) as i64;
    format!("{}:{:02}", m / 60, m % 60)
}


#[cfg(test)]
pub(crate) mod tests;
