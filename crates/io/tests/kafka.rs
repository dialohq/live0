mod common;

use common::contract::{
    from, number, numbers_schema, offsets, stream_contract, subscribe, subscribe_err,
};
use common::{KafkaHarness, Topic, collect};
use io::Error;
use std::collections::BTreeMap;

stream_contract!(
    KafkaHarness::default(),
    #[ignore = "needs the dnvr stack, run with `dnvr io-tests`"]
);

#[tokio::test]
#[ignore = "needs the dnvr stack, run with `dnvr io-tests`"]
async fn watermark_below_low_is_an_error() {
    let h = KafkaHarness::default();
    let topic = Topic::new(1).await;
    for n in 0..5 {
        topic.produce(0, Some(&format!("{{\"n\": {n}}}"))).await;
    }
    topic.trim(0, 3).await;
    let schemas = BTreeMap::from([(topic.name.clone(), numbers_schema())]);
    let err = subscribe_err(&h, &from(&topic.name, &[(0, 1)]), &schemas).await;
    assert!(
        matches!(
            err,
            Error::OffsetOutOfRange {
                partition: 0,
                offset: 1,
                low: 3,
                high: 5,
                ..
            }
        ),
        "{err:?}"
    );
    let mut events = subscribe(&h, &from(&topic.name, &[(0, 3)]), &schemas).await;
    assert_eq!(offsets(&collect(events.as_mut(), 2).await), [3, 4]);
    topic.drop().await;
}

#[tokio::test]
#[ignore = "needs the dnvr stack, run with `dnvr io-tests`"]
async fn raw_payloads_keep_their_offset() {
    let h = KafkaHarness::default();
    let topic = Topic::new(1).await;
    let payloads = [Some("{nope"), None, Some(r#"{"n": 007}"#)];
    for payload in payloads {
        topic.produce(0, payload).await;
    }
    let schemas = BTreeMap::from([(topic.name.clone(), numbers_schema())]);
    let mut events = subscribe(&h, &from(&topic.name, &[(0, 0)]), &schemas).await;
    let got = collect(events.as_mut(), payloads.len()).await;
    assert_eq!(offsets(&got), [0, 1, 2]);
    assert_eq!(
        got.iter().map(number).collect::<Vec<_>>(),
        [None, None, Some(7)]
    );
    topic.drop().await;
}
