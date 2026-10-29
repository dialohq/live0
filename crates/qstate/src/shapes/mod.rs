mod append;

/// One private implementation per `compiler::Shape`; wrapping shapes hold their inner one.
pub(crate) enum ShapeState {
    Append(append::Append),
}
