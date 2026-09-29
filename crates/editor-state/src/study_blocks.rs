//! Fenced blocks for covering a hard subject in little space.
//!
//! Three fences, each a small plain-text format that renders as a widget
//! when the caret is elsewhere and edits as source when it is inside:
//!
//! - ```` ```readings ```` — the positions on a contested question, side
//!   by side: what each says, what counts for and against it, who holds
//!   it, and (optionally) where the page lands.
//! - ```` ```timeline ```` — `date | event` lines as a vertical timeline.
//! - ```` ```map ```` — places with coordinates and routes between them,
//!   drawn to scale as an SVG.
//!
//! Text inside renders inline markdown with the vault — links, emphasis,
//! scripture and source badges work as anywhere else.

use std::fmt::Write as _;

use crate::markdown::{VaultLookup, render_table_cell};

/// Is `info` one of this module's fences?
#[must_use]
pub fn is_study_fence(info: &str) -> bool {
    ["readings", "timeline", "map"]
        .iter()
        .any(|l| info.eq_ignore_ascii_case(l))
}

/// The widget HTML for a study fence's `body`.
///
/// `None` when the body has nothing to draw (the source then shows as a
/// code block). `focus` is the body's offset: a click on the widget away
/// from its links puts the caret there to edit it.
#[must_use]
pub fn render(
    info: &str,
    body: &str,
    focus: usize,
    vault: Option<&dyn VaultLookup>,
) -> Option<String> {
    let inner = if info.eq_ignore_ascii_case("readings") {
        readings(body, vault)?
    } else if info.eq_ignore_ascii_case("timeline") {
        timeline(body, vault)?
    } else {
        map(body)?
    };
    Some(format!(
        r#"<div class="md-study md-study-{kind}" data-focus-pos="{focus}">{inner}</div>"#,
        kind = info.to_ascii_lowercase()
    ))
}

fn inline(text: &str, vault: Option<&dyn VaultLookup>) -> String {
    render_table_cell(text.trim(), vault)
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ── Readings ────────────────────────────────────────────────────────

#[derive(Default)]
struct Reading {
    name: String,
    says: Vec<String>,
    pro: Vec<String>,
    con: Vec<String>,
    held: Vec<String>,
}

/// ```text
/// Who are the “gods” of Psalm 82?        ← optional question
/// ## Human judges
/// Israel's rulers, called gods as judges in God's name.
/// + Fits the charge of injustice.
/// - “Die like men” only lands on someone who is not a man.
/// held: Calvin; much of the church's commentary
/// ## Divine beings
/// …
/// verdict: The divine-council reading, after Ugarit.
/// ```
fn readings(body: &str, vault: Option<&dyn VaultLookup>) -> Option<String> {
    let mut question = Vec::new();
    let mut verdict = None;
    let mut all: Vec<Reading> = Vec::new();
    for line in body.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(name) = line.strip_prefix("## ") {
            all.push(Reading {
                name: name.trim().to_owned(),
                ..Reading::default()
            });
            continue;
        }
        if let Some(v) = strip_key(line, "verdict") {
            verdict = Some(v.to_owned());
            continue;
        }
        let Some(r) = all.last_mut() else {
            question.push(line.to_owned());
            continue;
        };
        if let Some(p) = line.strip_prefix("+ ") {
            r.pro.push(p.to_owned());
        } else if let Some(c) = line.strip_prefix("- ") {
            r.con.push(c.to_owned());
        } else if let Some(h) = strip_key(line, "held") {
            r.held.push(h.to_owned());
        } else {
            r.says.push(line.to_owned());
        }
    }
    if all.is_empty() {
        return None;
    }
    let mut out = String::new();
    if !question.is_empty() {
        let _ = write!(
            out,
            r#"<div class="md-readings-q">{}</div>"#,
            inline(&question.join(" "), vault)
        );
    }
    let _ = write!(out, r#"<div class="md-readings-grid" style="--n:{}">"#, all.len().min(3));
    for r in &all {
        let _ = write!(
            out,
            r#"<div class="md-reading"><div class="md-reading-name">{}</div>"#,
            inline(&r.name, vault)
        );
        if !r.says.is_empty() {
            let _ = write!(
                out,
                r#"<div class="md-reading-says">{}</div>"#,
                inline(&r.says.join(" "), vault)
            );
        }
        for (class, mark, items) in [("pro", "+", &r.pro), ("con", "−", &r.con)] {
            if items.is_empty() {
                continue;
            }
            let _ = write!(out, r#"<ul class="md-reading-{class}">"#);
            for it in items {
                let _ = write!(
                    out,
                    r#"<li><span class="md-reading-mark">{mark}</span><span>{}</span></li>"#,
                    inline(it, vault)
                );
            }
            out.push_str("</ul>");
        }
        if !r.held.is_empty() {
            let _ = write!(
                out,
                r#"<div class="md-reading-held"><span>Held by</span> {}</div>"#,
                inline(&r.held.join("; "), vault)
            );
        }
        out.push_str("</div>");
    }
    out.push_str("</div>");
    if let Some(v) = verdict {
        let _ = write!(
            out,
            r#"<div class="md-readings-verdict"><span>Where this lands</span> {}</div>"#,
            inline(&v, vault)
        );
    }
    Some(out)
}

/// `key: value` (case-insensitive key) → `value`.
fn strip_key<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let (k, v) = line.split_once(':')?;
    k.trim().eq_ignore_ascii_case(key).then(|| v.trim())
}

// ── Timeline ────────────────────────────────────────────────────────

/// ```text
/// 1928 | A farmer's plough opens a tomb near Minet el-Beida.
/// 1929 | The dig at Ras Shamra; the first tablets.
/// ```
/// A line without a `|` continues the event above it.
fn timeline(body: &str, vault: Option<&dyn VaultLookup>) -> Option<String> {
    let mut rows: Vec<(String, String)> = Vec::new();
    for line in body.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some((date, event)) = line.split_once(" | ") {
            rows.push((date.trim().to_owned(), event.trim().to_owned()));
        } else if let Some(last) = rows.last_mut() {
            last.1.push(' ');
            last.1.push_str(line);
        }
    }
    if rows.is_empty() {
        return None;
    }
    let mut out = String::from(r#"<ol class="md-timeline">"#);
    for (date, event) in &rows {
        let _ = write!(
            out,
            r#"<li><span class="md-timeline-date">{}</span><span class="md-timeline-dot"></span><span class="md-timeline-event">{}</span></li>"#,
            esc(date),
            inline(event, vault)
        );
    }
    out.push_str("</ol>");
    Some(out)
}

// ── Map ─────────────────────────────────────────────────────────────

struct Place {
    name: String,
    lat: f64,
    lon: f64,
    note: String,
}

/// ```text
/// Jerusalem | 31.78, 35.22 | where it starts
/// Antioch | 36.20, 36.16
/// route: Jerusalem > Antioch > Cyprus
/// title: Paul's road west
/// ```
/// Drawn to scale (an equirectangular projection corrected for the
/// latitude), with a light graticule for bearings. No coastlines: this is
/// a diagram of where things are relative to each other, and says so.
fn map(body: &str) -> Option<String> {
    let (places, routes, title) = parse_map(body);
    if places.is_empty() {
        return None;
    }
    let svg = map_svg(&places, &routes, title.as_deref());
    let head = title.map_or_else(String::new, |t| {
        format!(r#"<div class="md-map-title">{}</div>"#, esc(&t))
    });
    let notes: Vec<String> = places
        .iter()
        .filter(|p| !p.note.is_empty())
        .map(|p| {
            format!(
                r#"<li><span class="md-map-note-name">{}</span> {}</li>"#,
                esc(&p.name),
                esc(&p.note)
            )
        })
        .collect();
    let notes = if notes.is_empty() {
        String::new()
    } else {
        format!(r#"<ul class="md-map-notes">{}</ul>"#, notes.concat())
    };
    Some(format!(
        r#"{head}{svg}<div class="md-map-scale">Positions to scale · no coastlines</div>{notes}"#
    ))
}

type MapSpec = (Vec<Place>, Vec<Vec<String>>, Option<String>);

fn parse_map(body: &str) -> MapSpec {
    let mut places: Vec<Place> = Vec::new();
    let mut routes: Vec<Vec<String>> = Vec::new();
    let mut title = None;
    for line in body.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(r) = strip_key(line, "route") {
            routes.push(r.split('>').map(|p| p.trim().to_owned()).collect());
            continue;
        }
        if let Some(t) = strip_key(line, "title") {
            title = Some(t.to_owned());
            continue;
        }
        let mut parts = line.split(" | ");
        let (Some(name), Some(coords)) = (parts.next(), parts.next()) else {
            continue;
        };
        let Some((lat, lon)) = coords.split_once(',') else {
            continue;
        };
        let (Ok(lat), Ok(lon)) = (lat.trim().parse::<f64>(), lon.trim().parse::<f64>()) else {
            continue;
        };
        places.push(Place {
            name: name.trim().to_owned(),
            lat,
            lon,
            note: parts.collect::<Vec<_>>().join(" | ").trim().to_owned(),
        });
    }
    (places, routes, title)
}

/// The places and routes, drawn to scale.
/// A label's box on the map, for keeping labels off each other.
#[derive(Clone, Copy)]
struct Label {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

impl Label {
    const fn overlaps(&self, o: &Self) -> bool {
        self.x0 < o.x1 && o.x0 < self.x1 && self.y0 < o.y1 && o.y0 < self.y1
    }
}

/// Where a place's name goes: right of its dot, else left, above, below
/// or a diagonal — the first spot that is clear of every label and dot
/// placed so far (then taken). Crowded clusters — the Aegean on Paul's
/// route — stay legible.
fn place_label(
    name: &str,
    px: f64,
    py: f64,
    width: f64,
    taken: &mut Vec<Label>,
) -> (f64, f64, &'static str) {
    let chars = f64::from(u32::try_from(name.chars().count()).unwrap_or(u32::MAX));
    let w = chars * 6.4;
    let h = 12.0;
    // (text x, baseline y, anchor) relative to the dot.
    let spots = [
        (8.0, 4.0, "start"),
        (-8.0, 4.0, "end"),
        (0.0, -9.0, "middle"),
        (0.0, 17.0, "middle"),
        (7.0, -7.0, "start"),
        (-7.0, -7.0, "end"),
        (7.0, 15.0, "start"),
        (-7.0, 15.0, "end"),
    ];
    let boxed = |(dx, dy, anchor): (f64, f64, &str)| {
        let x = px + dx;
        let x0 = match anchor {
            "end" => x - w,
            "middle" => x - w / 2.0,
            _ => x,
        };
        Label { x0, y0: py + dy - h + 2.0, x1: x0 + w, y1: py + dy + 2.0 }
    };
    let fits = |b: &Label| b.x0 >= 0.0 && b.x1 <= width;
    let chosen = spots
        .iter()
        .copied()
        .find(|&spot| {
            let b = boxed(spot);
            fits(&b) && !taken.iter().any(|t| t.overlaps(&b))
        })
        .or_else(|| spots.iter().copied().find(|&spot| fits(&boxed(spot))))
        .unwrap_or(spots[0]);
    taken.push(boxed(chosen));
    (px + chosen.0, py + chosen.1, chosen.2)
}

/// The frame a map is drawn in: degrees in, SVG units out.
struct Frame {
    lat_scale: f64,
    x0: f64,
    y1: f64,
    sx: f64,
    sy: f64,
    width: f64,
    height: f64,
}

impl Frame {
    fn fit(places: &[Place]) -> Self {
        let (min_lat, max_lat) = places
            .iter()
            .fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.lat), b.max(p.lat)));
        let (min_lon, max_lon) = places
            .iter()
            .fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.lon), b.max(p.lon)));
        let lat_scale = f64::midpoint(min_lat, max_lat).to_radians().cos().max(0.2);
        let pad = 1.5_f64;
        let (x0, x1) = ((min_lon - pad) * lat_scale, (max_lon + pad) * lat_scale);
        let (y0, y1) = (min_lat - pad, max_lat + pad);
        let span_x = (x1 - x0).max(0.1);
        let span_y = (y1 - y0).max(0.1);
        let width = 640.0_f64;
        let height = (width * span_y / span_x).clamp(200.0, 520.0);
        Self {
            lat_scale,
            x0,
            y1,
            sx: width / span_x,
            sy: height / span_y,
            width,
            height,
        }
    }

    fn x(&self, lon: f64) -> f64 {
        lon.mul_add(self.lat_scale, -self.x0) * self.sx
    }

    fn y(&self, lat: f64) -> f64 {
        (self.y1 - lat) * self.sy
    }

    fn at(&self, p: &Place) -> (f64, f64) {
        (self.x(p.lon), self.y(p.lat))
    }
}

/// The places and routes, drawn to scale, with a light graticule every
/// five degrees for bearing.
fn map_svg(places: &[Place], routes: &[Vec<String>], title: Option<&str>) -> String {
    let f = Frame::fit(places);
    let (width, height) = (f.width, f.height);
    let mut svg = format!(
        r#"<svg class="md-map-svg" viewBox="0 0 {width:.0} {height:.0}" role="img" aria-label="{}">"#,
        esc(title.unwrap_or("Map"))
    );
    svg.push_str(
        r#"<defs><marker id="md-map-arrow" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" class="md-map-arrowhead"/></marker></defs>"#,
    );
    for deg in (-180_i32..=180).step_by(5) {
        let x = f.x(f64::from(deg));
        if (0.0..=width).contains(&x) {
            let _ = write!(svg, r#"<line class="md-map-grid" x1="{x:.1}" y1="0" x2="{x:.1}" y2="{height:.0}"/>"#);
        }
    }
    for deg in (-90_i32..=90).step_by(5) {
        let y = f.y(f64::from(deg));
        if (0.0..=height).contains(&y) {
            let _ = write!(svg, r#"<line class="md-map-grid" x1="0" y1="{y:.1}" x2="{width:.0}" y2="{y:.1}"/>"#);
        }
    }
    for route in routes {
        let pts: Vec<(f64, f64)> = route
            .iter()
            .filter_map(|n| places.iter().find(|p| p.name.eq_ignore_ascii_case(n)))
            .map(|p| f.at(p))
            .collect();
        if pts.len() < 2 {
            continue;
        }
        let mut d = String::new();
        for (i, (x, y)) in pts.iter().enumerate() {
            let _ = write!(d, "{}{x:.1},{y:.1}", if i == 0 { "M" } else { " L" });
        }
        let _ = write!(
            svg,
            r#"<path class="md-map-route" d="{d}" marker-end="url(#md-map-arrow)"/>"#
        );
    }
    let mut taken: Vec<Label> = places
        .iter()
        .map(|p| {
            let (px, py) = f.at(p);
            Label { x0: px - 5.0, y0: py - 5.0, x1: px + 5.0, y1: py + 5.0 }
        })
        .collect();
    for p in places {
        let (px, py) = f.at(p);
        let (tx, ty, anchor) = place_label(&p.name, px, py, width, &mut taken);
        let tip = if p.note.is_empty() {
            String::new()
        } else {
            format!("<title>{}</title>", esc(&p.note))
        };
        let _ = write!(
            svg,
            r#"<g class="md-map-place">{tip}<circle cx="{px:.1}" cy="{py:.1}" r="4.5"/><text x="{tx:.1}" y="{ty:.1}" text-anchor="{anchor}">{}</text></g>"#,
            esc(&p.name),
        );
    }
    svg.push_str("</svg>");
    svg
}

#[cfg(test)]
mod tests {
    #[test]
    fn readings_render_side_by_side_with_their_case() {
        let body = "Who are the gods?\n## Human judges\nIsrael's rulers.\n+ Fits the charge\n- Die like men\nheld: Calvin\n## Divine beings\nThe council.\nverdict: Divine beings.\n";
        let html = super::render("readings", body, 10, None).expect("readings");
        assert!(html.contains(r#"data-focus-pos="10""#), "{html}");
        assert!(html.contains(r#"<div class="md-readings-q">Who are the gods?</div>"#), "{html}");
        assert_eq!(html.matches(r#"class="md-reading""#).count(), 2, "{html}");
        assert!(html.contains("md-reading-pro") && html.contains("md-reading-con"), "{html}");
        assert!(html.contains("<span>Held by</span> Calvin"), "{html}");
        assert!(html.contains("Where this lands"), "{html}");
        assert!(super::render("readings", "no sections here", 0, None).is_none());
    }

    #[test]
    fn a_timeline_is_dated_rows() {
        let html = super::render("timeline", "1928 | A tomb\n1929 | The dig\ncontinues here\n", 0, None)
            .expect("timeline");
        assert_eq!(html.matches("<li>").count(), 2, "{html}");
        assert!(html.contains("The dig continues here"), "{html}");
    }

    #[test]
    fn a_map_draws_places_and_routes_to_scale() {
        let body = "title: West\nJerusalem | 31.78, 35.22 | start\nRome | 41.9, 12.5\nroute: Jerusalem > Rome\n";
        let html = super::render("map", body, 0, None).expect("map");
        assert_eq!(html.matches("<circle").count(), 2, "{html}");
        assert!(html.contains(r#"class="md-map-route""#), "{html}");
        assert!(html.contains("md-map-title"), "{html}");
        // Rome is west of Jerusalem: drawn to its left.
        let x = |name: &str| -> f64 {
            let i = html.find(&format!(">{name}</text>")).unwrap();
            let c = html[..i].rfind("cx=\"").unwrap();
            html[c + 4..].split('"').next().unwrap().parse().unwrap()
        };
        assert!(x("Rome") < x("Jerusalem"));
    }
}
