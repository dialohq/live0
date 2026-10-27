//! Runs every case in `crates/types/tests/cases` through the dnvr stack (Redpanda, Kafka Connect with the
//! ClickHouse Sink, ClickHouse) and writes what the Sink did with each one to `crates/types/tests/results`.
//! Run it with the `decoder-cases` dnvr script while `dnvr up` is running.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value as Json};
use types::{HISTORY_FORMAT, HISTORY_SETTINGS};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const FAMILIES: &[&str] = &["string", "json", "int", "float", "bool"];
const DATABASE: &str = "decoder_cases";
const TOPIC_PREFIX: &str = "dc_";
const CONNECTOR: &str = "decoder-cases";
const DEV_CONNECTOR: &str = "clickhouse-sink";
// Each case gets its own topic and table, because the Sink rejects a whole insert batch when
// one record in it is bad; chunking bounds how many exist at once.
const CHUNK: usize = 200;
const CHUNK_TIMEOUT: Duration = Duration::from_secs(180);

struct Case {
    family: &'static str,
    ty: String,
    name: String,
    /// Raw JSON text of the column's value, `None` for an absent key.
    value: Option<String>,
}

enum Outcome {
    Accepted(Json),
    Rejected(String),
}

struct Stack {
    redpanda: String,
    connect: String,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("decoder-cases: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let stack = Stack {
        redpanda: std::env::var("REDPANDA_CONTAINER")?,
        connect: std::env::var("CONNECT_URL")?,
    };
    let cases = load_cases()?;
    let dev_config = stack.connect_get(&format!("/connectors/{DEV_CONNECTOR}/config"))?;
    let run_id = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

    stack.cleanup()?;
    ch(&format!("CREATE DATABASE {DATABASE}"))?;
    let mut outcomes = Vec::with_capacity(cases.len());
    let chunks = cases.chunks(CHUNK).count();
    for (i, chunk) in cases.chunks(CHUNK).enumerate() {
        eprintln!(
            "decoder-cases: chunk {}/{chunks} ({} cases)",
            i + 1,
            chunk.len()
        );
        outcomes.extend(stack.run_chunk(
            chunk,
            &format!("{TOPIC_PREFIX}{run_id}_{i}"),
            &dev_config,
        )?);
    }
    let versions = stack.versions()?;
    stack.cleanup()?;

    write_results(&cases, &outcomes, &versions)
}

fn tests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../types/tests")
}

fn load_cases() -> Result<Vec<Case>> {
    let mut cases = Vec::new();
    for family in FAMILIES {
        let path = tests_dir().join("cases").join(format!("{family}.json"));
        let file: Json = serde_json::from_str(&fs::read_to_string(&path)?)?;
        let types = file["types"].as_array().ok_or("`types` must be an array")?;
        let values = file["cases"].as_array().ok_or("`cases` must be an array")?;
        for ty in types {
            let ty = ty.as_str().ok_or("types must be strings")?;
            for case in values {
                cases.push(Case {
                    family,
                    ty: ty.to_owned(),
                    name: case["name"]
                        .as_str()
                        .ok_or("case without a name")?
                        .to_owned(),
                    value: case["value"].as_str().map(str::to_owned),
                });
            }
        }
    }
    Ok(cases)
}

impl Stack {
    fn run_chunk(&self, chunk: &[Case], prefix: &str, dev_config: &Json) -> Result<Vec<Outcome>> {
        let topics: Vec<String> = (0..chunk.len()).map(|j| format!("{prefix}_{j}")).collect();
        let dlq = format!("{prefix}_dlq");

        let mut ddl = String::new();
        for (case, topic) in chunk.iter().zip(&topics) {
            ddl += &format!(
                "CREATE TABLE {DATABASE}.{topic} (case String, v {}) ENGINE = MergeTree ORDER BY tuple();\n",
                case.ty
            );
        }
        ch_stdin(&ddl)?;
        let mut create = vec!["topic", "create"];
        create.extend(topics.iter().map(String::as_str));
        create.push(&dlq);
        self.rpk(&create, None)?;

        let mut config = dev_config
            .as_object()
            .ok_or("dev connector config isn't an object")?
            .clone();
        config.remove("name");
        for (key, value) in [
            ("topics", topics.join(",")),
            ("database", DATABASE.to_owned()),
            ("zkPath", format!("/kafka-connect-{CONNECTOR}")),
            ("zkDatabase", DATABASE.to_owned()),
            ("errors.tolerance", "all".to_owned()),
            ("errors.deadletterqueue.topic.name", dlq.clone()),
            (
                "errors.deadletterqueue.topic.replication.factor",
                "1".to_owned(),
            ),
            (
                "errors.deadletterqueue.context.headers.enable",
                "true".to_owned(),
            ),
        ] {
            config.insert(key.to_owned(), Json::String(value));
        }
        self.connect_put(
            &format!("/connectors/{CONNECTOR}/config"),
            &Json::Object(config),
        )?;

        let mut records = Vec::new();
        for (case, topic) in chunk.iter().zip(&topics) {
            let name = Json::String(case.name.clone());
            let event = match &case.value {
                Some(v) => format!("{{\"case\":{name},\"v\":{v}}}"),
                None => format!("{{\"case\":{name}}}"),
            };
            records.extend(format!("{topic} {} {event}\n", event.len()).into_bytes());
        }
        self.rpk(&["topic", "produce", "-f", "%t %V %v\n"], Some(&records))?;

        let (accepted, rejected) = self.wait(prefix, &topics, &dlq)?;
        let history = fetch_history(&accepted)?;
        self.connect_delete(&format!("/connectors/{CONNECTOR}"))?;

        topics
            .iter()
            .map(|topic| match (history.get(topic), rejected.get(topic)) {
                (Some(v), None) => Ok(Outcome::Accepted(v.clone())),
                (None, Some(e)) => Ok(Outcome::Rejected(e.clone())),
                _ => Err(format!("{topic} is both accepted and rejected").into()),
            })
            .collect()
    }

    /// Until every topic's record is either a row in its table or in the dead letter queue.
    fn wait(
        &self,
        prefix: &str,
        topics: &[String],
        dlq: &str,
    ) -> Result<(BTreeSet<String>, BTreeMap<String, String>)> {
        let start = Instant::now();
        loop {
            let accepted: BTreeSet<String> = ch(&format!(
                "SELECT name FROM system.tables WHERE database = '{DATABASE}' \
                 AND startsWith(name, '{prefix}_') AND total_rows > 0"
            ))?
            .lines()
            .map(str::to_owned)
            .collect();
            let rejected = self.dead_letters(dlq)?;
            let pending: Vec<&str> = topics
                .iter()
                .filter(|t| !accepted.contains(*t) && !rejected.contains_key(*t))
                .map(String::as_str)
                .collect();
            if pending.is_empty() {
                return Ok((accepted, rejected));
            }
            if start.elapsed() > CHUNK_TIMEOUT {
                return Err(format!(
                    "no outcome for {} topics: {}",
                    pending.len(),
                    pending.join(", ")
                )
                .into());
            }
            sleep(Duration::from_secs(1));
        }
    }

    /// Topic of each dead-lettered record → the root cause the Sink or converter reported.
    fn dead_letters(&self, dlq: &str) -> Result<BTreeMap<String, String>> {
        let out = self.rpk(
            &["topic", "consume", dlq, "-o", ":end", "--format", "json"],
            None,
        )?;
        let mut rejected = BTreeMap::new();
        for record in serde_json::Deserializer::from_str(&out).into_iter::<Json>() {
            let record = record?;
            let headers: BTreeMap<&str, &str> = record["headers"]
                .as_array()
                .ok_or("dead letter without headers")?
                .iter()
                .filter_map(|h| Some((h["key"].as_str()?, h["value"].as_str()?)))
                .collect();
            let topic = headers
                .get("__connect.errors.topic")
                .ok_or("dead letter without a topic header")?;
            let error = headers
                .get("__connect.errors.exception.stacktrace")
                .and_then(|trace| root_cause(trace))
                .or_else(|| headers.get("__connect.errors.exception.message").copied())
                .unwrap_or_default();
            rejected.insert((*topic).to_owned(), error.to_owned());
        }
        Ok(rejected)
    }

    fn versions(&self) -> Result<Json> {
        let plugins = self.connect_get("/connector-plugins")?;
        let sink = plugins
            .as_array()
            .and_then(|ps| {
                ps.iter().find(|p| {
                    p["class"]
                        .as_str()
                        .is_some_and(|c| c.ends_with("ClickHouseSinkConnector"))
                })
            })
            .ok_or("ClickHouse Sink plugin not found")?;
        let mut versions = Map::new();
        versions.insert("clickhouse".into(), ch("SELECT version()")?.trim().into());
        versions.insert(
            "kafka_connect".into(),
            self.connect_get("/")?["version"].clone(),
        );
        versions.insert("clickhouse_sink".into(), sink["version"].clone());
        Ok(Json::Object(versions))
    }

    fn cleanup(&self) -> Result<()> {
        self.connect_delete(&format!("/connectors/{CONNECTOR}"))?;
        ch(&format!("DROP DATABASE IF EXISTS {DATABASE} SYNC"))?;
        self.rpk(
            &["topic", "delete", "-r", &format!("^{TOPIC_PREFIX}.*")],
            None,
        )?;
        Ok(())
    }

    fn rpk(&self, args: &[&str], stdin: Option<&[u8]>) -> Result<String> {
        let mut cmd = Command::new("docker");
        cmd.args(["exec", "-i", &self.redpanda, "rpk"]).args(args);
        output(cmd, stdin)
    }

    fn connect_get(&self, path: &str) -> Result<Json> {
        let mut cmd = Command::new("curl");
        cmd.args(["-sSf", &format!("{}{path}", self.connect)]);
        Ok(serde_json::from_str(&output(cmd, None)?)?)
    }

    fn connect_put(&self, path: &str, body: &Json) -> Result<()> {
        let mut cmd = Command::new("curl");
        cmd.args([
            "-sSf",
            "-X",
            "PUT",
            "-H",
            "Content-Type: application/json",
            "--data-binary",
            "@-",
        ])
        .arg(format!("{}{path}", self.connect));
        output(cmd, Some(body.to_string().as_bytes()))?;
        Ok(())
    }

    /// Deleting something that doesn't exist is fine.
    fn connect_delete(&self, path: &str) -> Result<()> {
        let mut cmd = Command::new("curl");
        cmd.args([
            "-sS",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            "-X",
            "DELETE",
        ])
        .arg(format!("{}{path}", self.connect));
        match output(cmd, None)?.as_str() {
            "204" | "404" => Ok(()),
            code => Err(format!("DELETE {path} returned HTTP {code}").into()),
        }
    }
}

/// The accepted topics' `v`, fetched exactly as the application fetches history.
fn fetch_history(topics: &BTreeSet<String>) -> Result<BTreeMap<String, Json>> {
    let settings = HISTORY_SETTINGS
        .iter()
        .map(|(k, v)| format!("{k} = {v}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut queries = String::new();
    for topic in topics {
        queries += &format!(
            "SELECT '{topic}' AS topic, v FROM {DATABASE}.{topic} SETTINGS {settings} FORMAT {HISTORY_FORMAT};\n"
        );
    }
    let mut history = BTreeMap::new();
    for line in ch_stdin(&queries)?.lines().filter(|l| !l.trim().is_empty()) {
        let row: Json = serde_json::from_str(line)?;
        let topic = row["topic"]
            .as_str()
            .ok_or("history row without topic")?
            .to_owned();
        if history.insert(topic.clone(), row["v"].clone()).is_some() {
            return Err(format!("{topic} has more than one row").into());
        }
    }
    Ok(history)
}

/// The first line of the innermost `Caused by:`, without the frames and the server version.
fn root_cause(trace: &str) -> Option<&str> {
    trace
        .lines()
        .filter_map(|l| l.strip_prefix("Caused by: "))
        .next_back()
}

fn write_results(cases: &[Case], outcomes: &[Outcome], versions: &Json) -> Result<()> {
    let dir = tests_dir().join("results");
    fs::create_dir_all(&dir)?;
    for family in FAMILIES {
        let mut out = String::new();
        for (case, outcome) in cases
            .iter()
            .zip(outcomes)
            .filter(|(c, _)| c.family == *family)
        {
            let event = case
                .value
                .as_ref()
                .map_or(Json::Null, |v| Json::String(v.clone()));
            let (accepted, detail) = match outcome {
                Outcome::Accepted(v) => ("true", format!("\"history\":{v}")),
                Outcome::Rejected(e) => ("false", format!("\"error\":{}", Json::String(e.clone()))),
            };
            out += &format!(
                "{{\"type\":{},\"case\":{},\"event\":{event},\"accepted\":{accepted},{detail}}}\n",
                Json::String(case.ty.clone()),
                Json::String(case.name.clone()),
            );
        }
        fs::write(dir.join(format!("{family}.jsonl")), out)?;
    }
    fs::write(
        dir.join("versions.json"),
        serde_json::to_string_pretty(versions)? + "\n",
    )?;
    Ok(())
}

fn ch(query: &str) -> Result<String> {
    let mut cmd = Command::new("ch");
    cmd.args(["-q", query]);
    output(cmd, None)
}

fn ch_stdin(queries: &str) -> Result<String> {
    let mut cmd = Command::new("ch");
    cmd.arg("--multiquery");
    output(cmd, Some(queries.as_bytes()))
}

fn output(mut cmd: Command, stdin: Option<&[u8]>) -> Result<String> {
    cmd.stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(input)?;
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        return Err(format!(
            "{cmd:?} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )
        .into());
    }
    Ok(String::from_utf8(out.stdout)?)
}
