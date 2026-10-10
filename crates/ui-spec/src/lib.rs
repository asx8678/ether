// New in ether: the generative UI spec drawn by the `render_ui` tool.
//! A view is a tree of [`Node`]s. The model sends it as JSON; [`parse`] turns it into a `Node`
//! and checks the limits, with errors worded for the model (they go back as the tool result,
//! so it can fix the spec and call again). The engine only parses; front-ends draw it.

use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde_json::Value;

/// Deepest nesting of `column`/`row` containers.
const MAX_DEPTH: usize = 8;
/// Most nodes in one view.
const MAX_NODES: usize = 200;
const MAX_TABLE_ROWS: usize = 200;
const MAX_TABLE_COLUMNS: usize = 12;

/// How the model is told to write a spec; part of the `render_ui` tool description.
pub const SPEC_GUIDE: &str = r#"A spec is one node. Every node is an object with a "type":
- {"type":"column","children":[nodes],"gap":0} stacks children vertically; gap = blank rows between them.
- {"type":"row","children":[nodes],"gap":1} places children side by side in equal-width columns; gap = blank columns.
- {"type":"text","text":"...","bold":false,"dim":false,"italic":false,"color":"green"} plain text, wrapped.
- {"type":"markdown","markdown":"..."} rendered markdown.
- {"type":"table","title":"optional","headers":["A","B"],"rows":[["1","2"]]} every row has one cell per header; cells are strings, numbers or booleans.
- {"type":"gauge","label":"CPU","value":63,"max":100,"color":"yellow"} a horizontal bar filled to value/max (max defaults to 100).
Colors: black, red, green, yellow, blue, magenta, cyan, gray, white. Only "type" and the content fields are required."#;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Node {
    Column {
        children: Vec<Node>,
        #[serde(default)]
        gap: u16,
    },
    Row {
        children: Vec<Node>,
        #[serde(default = "default_row_gap")]
        gap: u16,
    },
    Text {
        text: String,
        #[serde(default)]
        bold: bool,
        #[serde(default)]
        dim: bool,
        #[serde(default)]
        italic: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<Color>,
    },
    Markdown {
        markdown: String,
    },
    Table {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        headers: Vec<String>,
        rows: Vec<Vec<Cell>>,
    },
    Gauge {
        label: String,
        value: f64,
        #[serde(default = "default_gauge_max")]
        max: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<Color>,
    },
}

fn default_row_gap() -> u16 {
    1
}

fn default_gauge_max() -> f64 {
    100.0
}

/// A gauge's fill, from 0 to 1.
pub fn gauge_ratio(value: f64, max: f64) -> f64 {
    (value / max).clamp(0.0, 1.0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Color {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    #[serde(alias = "grey")]
    Gray,
    White,
}

/// A table cell. The model may send a string, number, boolean or null; it is kept as text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Cell(pub String);

impl<'de> Deserialize<'de> for Cell {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match Value::deserialize(deserializer)? {
            Value::String(text) => Ok(Cell(text)),
            Value::Number(number) => Ok(Cell(number.to_string())),
            Value::Bool(flag) => Ok(Cell(flag.to_string())),
            Value::Null => Ok(Cell(String::new())),
            other => Err(serde::de::Error::custom(format!(
                "a table cell must be a string, number or boolean, got {other}"
            ))),
        }
    }
}

/// Parses and checks a spec. A spec sent as a JSON string is parsed first. The error names the
/// failing node by path, e.g. `spec.children[1]: missing field `rows``.
pub fn parse(value: &Value) -> Result<Node, String> {
    let parsed;
    let value = match value {
        Value::String(text) => {
            parsed = serde_json::from_str::<Value>(text)
                .map_err(|err| format!("spec: not a JSON object: {err}"))?;
            &parsed
        }
        other => other,
    };
    let node = parse_at(value, "spec")?;
    let mut count = 0;
    check(&node, "spec", 0, &mut count)?;
    Ok(node)
}

fn parse_at(value: &Value, path: &str) -> Result<Node, String> {
    serde_json::from_value::<Node>(value.clone()).map_err(|err| {
        // Point at the deepest failing child rather than the whole container.
        if let Some(children) = value.get("children").and_then(Value::as_array) {
            for (index, child) in children.iter().enumerate() {
                if let Err(child_err) = parse_at(child, &format!("{path}.children[{index}]")) {
                    return child_err;
                }
            }
        }
        format!("{path}: {err}")
    })
}

fn check(node: &Node, path: &str, depth: usize, count: &mut usize) -> Result<(), String> {
    *count += 1;
    if *count > MAX_NODES {
        return Err(format!("spec: more than {MAX_NODES} nodes"));
    }
    match node {
        Node::Column { children, .. } | Node::Row { children, .. } => {
            if depth >= MAX_DEPTH {
                return Err(format!("{path}: nested deeper than {MAX_DEPTH} containers"));
            }
            if children.is_empty() {
                return Err(format!("{path}: `children` is empty"));
            }
            for (index, child) in children.iter().enumerate() {
                check(
                    child,
                    &format!("{path}.children[{index}]"),
                    depth + 1,
                    count,
                )?;
            }
        }
        Node::Text { .. } | Node::Markdown { .. } => {}
        Node::Table { headers, rows, .. } => {
            if headers.is_empty() {
                return Err(format!("{path}: `headers` is empty"));
            }
            if headers.len() > MAX_TABLE_COLUMNS {
                return Err(format!(
                    "{path}: {} columns; at most {MAX_TABLE_COLUMNS}",
                    headers.len()
                ));
            }
            if rows.len() > MAX_TABLE_ROWS {
                return Err(format!(
                    "{path}: {} rows; at most {MAX_TABLE_ROWS}",
                    rows.len()
                ));
            }
            if let Some((index, row)) = rows
                .iter()
                .enumerate()
                .find(|(_, row)| row.len() != headers.len())
            {
                return Err(format!(
                    "{path}.rows[{index}]: {} cells but {} headers; every row needs one cell per header",
                    row.len(),
                    headers.len()
                ));
            }
        }
        Node::Gauge { value, max, .. } => {
            if !value.is_finite() {
                return Err(format!("{path}: `value` must be a finite number"));
            }
            if !max.is_finite() || *max <= 0.0 {
                return Err(format!("{path}: `max` must be a number above 0"));
            }
        }
    }
    Ok(())
}
