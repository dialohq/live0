use std::collections::BTreeMap;

use io::{Error, EventStream, PartitionOffsets};
use serde_json::{Value as Json, json};
use types::{Column, DataType, Event, Offset, Partition, Schema, Value, Watermark};

use super::{Harness, assert_quiet, collect, unique};

/// One `#[tokio::test]` per contract case, each running against a fresh `$harness`.
macro_rules! stream_contract {
    ($harness:expr $(, #[$attr:meta])*) => {
        $crate::common::contract::stream_contract!(
            @each $harness, [$(#[$attr])*],
            partitions_report_low_and_high,
            missing_topic_is_an_error,
            starts_exactly_at_watermark,
            waits_at_high_for_new_records,
            every_offset_once_in_order,
            reads_only_partitions_in_watermark,
            decodes_each_topic_by_its_schema,
            undecodable_records_keep_their_offset,
            watermark_above_high_is_an_error,
            unknown_partition_is_an_error,
            topic_without_schema_is_an_error
        );
    };
    (@each $harness:expr, $attrs:tt, $($name:ident),*) => {
        $($crate::common::contract::stream_contract!(@one $name, $harness, $attrs);)*
    };
    (@one $name:ident, $harness:expr, [$(#[$attr:meta])*]) => {
        #[tokio::test]
        $(#[$attr])*
        async fn $name() {
            let harness = $harness;
            $crate::common::contract::$name(&harness).await;
            $crate::common::Harness::cleanup(&harness).await;
        }
    };
}
pub(crate) use stream_contract;

pub fn numbers_schema() -> Schema {
    Schema {
        columns: vec![Column::new("n", DataType::parse("UInt64"))],
    }
}

fn numbers(topic: &str) -> BTreeMap<String, Schema> {
    BTreeMap::from([(topic.to_owned(), numbers_schema())])
}

pub fn from(topic: &str, offsets: &[(Partition, Offset)]) -> Watermark {
    let mut watermark = Watermark::new();
    for (partition, offset) in offsets {
        watermark.set(topic, *partition, *offset);
    }
    watermark
}

async fn produce_numbers(
    h: &impl Harness,
    topic: &str,
    partition: Partition,
    numbers: impl Iterator<Item = i64>,
) {
    for n in numbers {
        h.produce(topic, partition, &json!({ "n": n })).await;
    }
}

pub async fn subscribe(
    h: &impl Harness,
    from: &Watermark,
    schemas: &BTreeMap<String, Schema>,
) -> Box<dyn EventStream> {
    match h.source().subscribe(from, schemas).await {
        Ok(events) => events,
        Err(err) => panic!("subscribe failed: {err:?}"),
    }
}

pub async fn subscribe_err(
    h: &impl Harness,
    from: &Watermark,
    schemas: &BTreeMap<String, Schema>,
) -> Error {
    match h.source().subscribe(from, schemas).await {
        Ok(_) => panic!("subscribe succeeded"),
        Err(err) => err,
    }
}

pub fn number(event: &Event) -> Option<i128> {
    match event.row.as_deref() {
        Some([Value::Int(n)]) => Some(*n),
        _ => None,
    }
}

pub fn offsets(events: &[Event]) -> Vec<Offset> {
    events.iter().map(|e| e.offset).collect()
}

pub async fn partitions_report_low_and_high(h: &impl Harness) {
    let topic = h.topic(2).await;
    produce_numbers(h, &topic, 0, 0..5).await;
    let offsets = |partition, high| PartitionOffsets {
        partition,
        low: 0,
        high,
    };
    assert_eq!(
        h.source().partitions(&topic).await.unwrap(),
        vec![offsets(0, 5), offsets(1, 0)]
    );
}

pub async fn missing_topic_is_an_error(h: &impl Harness) {
    let (source, name) = (h.source(), unique("missing"));
    for _ in 0..2 {
        let err = source.partitions(&name).await.unwrap_err();
        assert!(
            matches!(&err, Error::UnknownTopic(t) if *t == name),
            "{err:?}"
        );
    }
}

pub async fn starts_exactly_at_watermark(h: &impl Harness) {
    let topic = h.topic(1).await;
    produce_numbers(h, &topic, 0, 0..10).await;
    let mut events = subscribe(h, &from(&topic, &[(0, 4)]), &numbers(&topic)).await;
    let got = collect(events.as_mut(), 6).await;
    assert_eq!(offsets(&got), (4..10).collect::<Vec<_>>());
    assert_eq!(
        got.iter().map(number).collect::<Vec<_>>(),
        (4..10).map(Some).collect::<Vec<_>>()
    );
    assert!(got.iter().all(|e| e.topic == topic && e.partition == 0));
    assert_quiet(events.as_mut()).await;
}

pub async fn waits_at_high_for_new_records(h: &impl Harness) {
    let topic = h.topic(1).await;
    produce_numbers(h, &topic, 0, 0..3).await;
    let mut events = subscribe(h, &from(&topic, &[(0, 3)]), &numbers(&topic)).await;
    assert_quiet(events.as_mut()).await;
    produce_numbers(h, &topic, 0, 3..4).await;
    let got = collect(events.as_mut(), 1).await;
    assert_eq!((got[0].offset, number(&got[0])), (3, Some(3)));
}

pub async fn every_offset_once_in_order(h: &impl Harness) {
    let topic = h.topic(3).await;
    for partition in 0..3 {
        produce_numbers(h, &topic, partition, 0..150).await;
    }
    let starts = [(0, 0), (1, 50), (2, 150)];
    let mut events = subscribe(h, &from(&topic, &starts), &numbers(&topic)).await;
    for partition in 0..3 {
        produce_numbers(h, &topic, partition, 150..300).await;
    }
    let got = collect(events.as_mut(), 300 + 250 + 150).await;
    for (partition, start) in starts {
        let got: Vec<Event> = got
            .iter()
            .filter(|e| e.partition == partition)
            .cloned()
            .collect();
        assert_eq!(
            offsets(&got),
            (start..300).collect::<Vec<_>>(),
            "partition {partition}"
        );
    }
    assert_quiet(events.as_mut()).await;
}

pub async fn reads_only_partitions_in_watermark(h: &impl Harness) {
    let topic = h.topic(2).await;
    produce_numbers(h, &topic, 0, 0..3).await;
    produce_numbers(h, &topic, 1, 0..3).await;
    let mut events = subscribe(h, &from(&topic, &[(0, 0)]), &numbers(&topic)).await;
    let got = collect(events.as_mut(), 3).await;
    assert!(got.iter().all(|e| e.partition == 0), "{got:?}");
    assert_quiet(events.as_mut()).await;
}

pub async fn decodes_each_topic_by_its_schema(h: &impl Harness) {
    let (a, b) = (h.topic(1).await, h.topic(1).await);
    h.produce(&a, 0, &json!({"n": 1, "s": "x"})).await;
    h.produce(&b, 0, &json!({"n": 2, "s": "y"})).await;
    let schemas = BTreeMap::from([
        (a.clone(), numbers_schema()),
        (
            b.clone(),
            Schema {
                columns: vec![Column::new("s", DataType::parse("String"))],
            },
        ),
    ]);
    let mut watermark = from(&a, &[(0, 0)]);
    watermark.extend(from(&b, &[(0, 0)]));
    let mut events = subscribe(h, &watermark, &schemas).await;
    let rows: BTreeMap<String, _> = collect(events.as_mut(), 2)
        .await
        .into_iter()
        .map(|e| (e.topic, e.row))
        .collect();
    assert_eq!(rows[&a], Some(vec![Value::Int(1)]));
    assert_eq!(rows[&b], Some(vec![Value::String("y".to_owned())]));
}

pub async fn undecodable_records_keep_their_offset(h: &impl Harness) {
    let topic = h.topic(1).await;
    let values: [Json; 4] = [
        json!({"n": 1}),
        json!([1]),
        json!({"n": "x"}),
        json!({"n": 4}),
    ];
    for value in &values {
        h.produce(&topic, 0, value).await;
    }
    let mut events = subscribe(h, &from(&topic, &[(0, 0)]), &numbers(&topic)).await;
    let got = collect(events.as_mut(), values.len()).await;
    assert_eq!(offsets(&got), [0, 1, 2, 3]);
    assert_eq!(
        got.iter().map(number).collect::<Vec<_>>(),
        [Some(1), None, None, Some(4)]
    );
    assert!(got[1].row.is_none() && got[2].row.is_none());
}

pub async fn watermark_above_high_is_an_error(h: &impl Harness) {
    let topic = h.topic(1).await;
    produce_numbers(h, &topic, 0, 0..2).await;
    let err = subscribe_err(h, &from(&topic, &[(0, 3)]), &numbers(&topic)).await;
    assert!(
        matches!(
            err,
            Error::OffsetOutOfRange {
                offset: 3,
                high: 2,
                ..
            }
        ),
        "{err:?}"
    );
}

pub async fn unknown_partition_is_an_error(h: &impl Harness) {
    let topic = h.topic(1).await;
    let err = subscribe_err(h, &from(&topic, &[(1, 0)]), &numbers(&topic)).await;
    assert!(
        matches!(err, Error::UnknownPartition { partition: 1, .. }),
        "{err:?}"
    );
}

pub async fn topic_without_schema_is_an_error(h: &impl Harness) {
    let topic = h.topic(1).await;
    let err = subscribe_err(h, &from(&topic, &[(0, 0)]), &BTreeMap::new()).await;
    assert!(matches!(&err, Error::NoSchema(t) if *t == topic), "{err:?}");
}
