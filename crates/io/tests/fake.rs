mod common;

use common::FakeHarness;
use common::contract::stream_contract;

stream_contract!(FakeHarness::default());
