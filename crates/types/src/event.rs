use crate::Row;

pub struct Event {
    pub topic: String,
    pub partition: i32,
    pub offset: i64,
    pub value: Row,
}
