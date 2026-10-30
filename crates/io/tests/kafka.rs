mod common;

use std::collections::BTreeMap;

use common::{Topic, assert_quiet, collect, kafka_config, unique};
use io::{Error, EventStream, KafkaError, KafkaStream, PartitionOffsets, StreamSource};
use types::{Column, DataType, Event, Offset, Partition, Schema, Value, Watermark};

fn stream() -> KafkaStream {
    KafkaStream::new(&kafka_config()).unwrap()
}

fn schema(columns: &[(&str, &str)]) -> Schema {
    Schema {
        columns: columns
            .iter()
            .map(|(name, ty)| Column::new(*name, DataType::parse(ty)))
            .collect(),
    }
}

fn numbers(topic: &Topic) -> BTreeMap<String, Schema> {
    BTreeMap::from([(topic.name.clone(), schema(&[("n", "UInt64")]))])
}

fn from(topic: &Topic, offsets: &[(Partition, Offset)]) -> Watermark {
    let mut watermark = Watermark::new();
    for (partition, offset) in offsets {
        watermark.set(&topic.name, *partition, *offset);
    }
    watermark
}

async fn produce_numbers(topic: &Topic, partition: Partition, numbers: impl Iterator<Item = i64>) {
    for n in numbers {
        topic
            .produce(partition, Some(&format!("{{\"n\": {n}}}")))
            .await;
    }
}

async fn subscribe(from: &Watermark, schemas: &BTreeMap<String, Schema>) -> Box<dyn EventStream> {
    match stream().subscribe(from, schemas).await {
        Ok(events) => events,
        Err(err) => panic!("subscribe failed: {err:?}"),
    }
}

async fn subscribe_err(from: &Watermark, schemas: &BTreeMap<String, Schema>) -> Error {
    match stream().subscribe(from, schemas).await {
        Ok(_) => panic!("subscribe succeeded"),
        Err(err) => err,
    }
}

fn number(event: &Event) -> Option<i128> {
    match event.row.as_deref() {
        Some([Value::Int(n)]) => Some(*n),
        _ => None,
    }
}

#[tokio::test]
#[ignore = "needs the dnvr stack, run with `dnvr io-tests`"]
async fn partitions_report_low_and_high() {
    let topic = Topic::new(2).await;
    produce_numbers(&topic, 0, 0..5).await;
    assert_eq!(
        stream().partitions(&topic.name).await.unwrap(),
        vec![
            PartitionOffsets {
                partition: 0,
                low: 0,
                high: 5
            },
            PartitionOffsets {
                partition: 1,
                low: 0,
                high: 0
            },
        ]
    );
    topic.drop().await;
}

#[tokio::test]
#[ignore = "needs the dnvr stack, run with `dnvr io-tests`"]
async fn missing_topic_is_an_error() {
    let (stream, name) = (stream(), unique("io_test_missing"));
    for _ in 0..2 {
        let err = stream.partitions(&name).await.unwrap_err();
        assert!(
            matches!(&err, Error::Kafka(KafkaError::UnknownTopic(t)) if *t == name),
            "{err:?}"
        );
    }
}

#[tokio::test]
#[ignore = "needs the dnvr stack, run with `dnvr io-tests`"]
async fn starts_exactly_at_watermark() {
    let topic = Topic::new(1).await;
    produce_numbers(&topic, 0, 0..10).await;
    let mut events = subscribe(&from(&topic, &[(0, 4)]), &numbers(&topic)).await;
    let got = collect(events.as_mut(), 6).await;
    assert_eq!(
        got.iter().map(|e| e.offset).collect::<Vec<_>>(),
        (4..10).collect::<Vec<_>>()
    );
    assert_eq!(
        got.iter().map(number).collect::<Vec<_>>(),
        (4..10).map(Some).collect::<Vec<_>>()
    );
    assert!(
        got.iter()
            .all(|e| e.topic == topic.name && e.partition == 0)
    );
    assert_quiet(events.as_mut()).await;
    topic.drop().await;
}

#[tokio::test]
#[ignore = "needs the dnvr stack, run with `dnvr io-tests`"]
async fn waits_at_high_for_new_records() {
    let topic = Topic::new(1).await;
    produce_numbers(&topic, 0, 0..3).await;
    let mut events = subscribe(&from(&topic, &[(0, 3)]), &numbers(&topic)).await;
    assert_quiet(events.as_mut()).await;
    produce_numbers(&topic, 0, 3..4).await;
    let got = collect(events.as_mut(), 1).await;
    assert_eq!((got[0].offset, number(&got[0])), (3, Some(3)));
    topic.drop().await;
}

#[tokio::test]
#[ignore = "needs the dnvr stack, run with `dnvr io-tests`"]
async fn every_offset_once_in_order() {
    let topic = Topic::new(3).await;
    for partition in 0..3 {
        produce_numbers(&topic, partition, 0..150).await;
    }
    let starts = [(0, 0), (1, 50), (2, 150)];
    let mut events = subscribe(&from(&topic, &starts), &numbers(&topic)).await;
    for partition in 0..3 {
        produce_numbers(&topic, partition, 150..300).await;
    }
    let got = collect(events.as_mut(), 300 + 250 + 150).await;
    for (partition, start) in starts {
        let offsets: Vec<Offset> = got
            .iter()
            .filter(|e| e.partition == partition)
            .map(|e| e.offset)
            .collect();
        assert_eq!(
            offsets,
            (start..300).collect::<Vec<_>>(),
            "partition {partition}"
        );
    }
    assert_quiet(events.as_mut()).await;
    topic.drop().await;
}

#[tokio::test]
#[ignore = "needs the dnvr stack, run with `dnvr io-tests`"]
async fn reads_only_partitions_in_watermark() {
    let topic = Topic::new(2).await;
    produce_numbers(&topic, 0, 0..3).await;
    produce_numbers(&topic, 1, 0..3).await;
    let mut events = subscribe(&from(&topic, &[(0, 0)]), &numbers(&topic)).await;
    let got = collect(events.as_mut(), 3).await;
    assert!(got.iter().all(|e| e.partition == 0), "{got:?}");
    assert_quiet(events.as_mut()).await;
    topic.drop().await;
}

#[tokio::test]
#[ignore = "needs the dnvr stack, run with `dnvr io-tests`"]
async fn decodes_each_topic_by_its_schema() {
    let (a, b) = (Topic::new(1).await, Topic::new(1).await);
    a.produce(0, Some(r#"{"n": 1, "s": "x"}"#)).await;
    b.produce(0, Some(r#"{"n": 2, "s": "y"}"#)).await;
    let schemas = BTreeMap::from([
        (a.name.clone(), schema(&[("n", "UInt64")])),
        (b.name.clone(), schema(&[("s", "String")])),
    ]);
    let mut watermark = from(&a, &[(0, 0)]);
    watermark.extend(from(&b, &[(0, 0)]));
    let mut events = subscribe(&watermark, &schemas).await;
    let mut got = collect(events.as_mut(), 2).await;
    got.sort_by(|x, y| x.topic.cmp(&y.topic));
    let rows: BTreeMap<String, _> = got.into_iter().map(|e| (e.topic, e.row)).collect();
    assert_eq!(rows[&a.name], Some(vec![Value::Int(1)]));
    assert_eq!(rows[&b.name], Some(vec![Value::String("y".to_owned())]));
    a.drop().await;
    b.drop().await;
}

#[tokio::test]
#[ignore = "needs the dnvr stack, run with `dnvr io-tests`"]
async fn undecodable_records_keep_their_offset() {
    let topic = Topic::new(1).await;
    let payloads = [
        Some(r#"{"n": 1}"#),
        Some("{nope"),
        Some("[1]"),
        Some(r#"{"n": "x"}"#),
        None,
        Some(r#"{"n": 007}"#),
    ];
    for payload in payloads {
        topic.produce(0, payload).await;
    }
    let mut events = subscribe(&from(&topic, &[(0, 0)]), &numbers(&topic)).await;
    let got = collect(events.as_mut(), payloads.len()).await;
    assert_eq!(
        got.iter().map(|e| e.offset).collect::<Vec<_>>(),
        (0..6).collect::<Vec<_>>()
    );
    assert_eq!(
        got.iter().map(number).collect::<Vec<_>>(),
        [Some(1), None, None, None, None, Some(7)]
    );
    assert!(got[1..5].iter().all(|e| e.row.is_none()));
    topic.drop().await;
}

#[tokio::test]
#[ignore = "needs the dnvr stack, run with `dnvr io-tests`"]
async fn watermark_below_low_is_an_error() {
    let topic = Topic::new(1).await;
    produce_numbers(&topic, 0, 0..5).await;
    topic.trim(0, 3).await;
    let err = subscribe_err(&from(&topic, &[(0, 1)]), &numbers(&topic)).await;
    assert!(
        matches!(
            err,
            Error::Kafka(KafkaError::OffsetOutOfRange {
                partition: 0,
                offset: 1,
                low: 3,
                high: 5,
                ..
            })
        ),
        "{err:?}"
    );
    let mut events = subscribe(&from(&topic, &[(0, 3)]), &numbers(&topic)).await;
    let got = collect(events.as_mut(), 2).await;
    assert_eq!(got.iter().map(|e| e.offset).collect::<Vec<_>>(), [3, 4]);
    topic.drop().await;
}

#[tokio::test]
#[ignore = "needs the dnvr stack, run with `dnvr io-tests`"]
async fn watermark_above_high_is_an_error() {
    let topic = Topic::new(1).await;
    produce_numbers(&topic, 0, 0..2).await;
    let err = subscribe_err(&from(&topic, &[(0, 3)]), &numbers(&topic)).await;
    assert!(
        matches!(
            err,
            Error::Kafka(KafkaError::OffsetOutOfRange {
                offset: 3,
                high: 2,
                ..
            })
        ),
        "{err:?}"
    );
    topic.drop().await;
}

#[tokio::test]
#[ignore = "needs the dnvr stack, run with `dnvr io-tests`"]
async fn unknown_partition_is_an_error() {
    let topic = Topic::new(1).await;
    let err = subscribe_err(&from(&topic, &[(1, 0)]), &numbers(&topic)).await;
    assert!(
        matches!(
            err,
            Error::Kafka(KafkaError::UnknownPartition { partition: 1, .. })
        ),
        "{err:?}"
    );
    topic.drop().await;
}

#[tokio::test]
#[ignore = "needs the dnvr stack, run with `dnvr io-tests`"]
async fn topic_without_schema_is_an_error() {
    let topic = Topic::new(1).await;
    let err = subscribe_err(&from(&topic, &[(0, 0)]), &BTreeMap::new()).await;
    assert!(
        matches!(&err, Error::Kafka(KafkaError::NoSchema(t)) if *t == topic.name),
        "{err:?}"
    );
    topic.drop().await;
}
