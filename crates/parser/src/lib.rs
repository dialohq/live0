use crate::{error::SyntaxError, lexer::tokenize};
pub use ast::Query;

pub mod ast;
pub mod error;
mod lexer;
mod parser;

pub fn parse(src: &str) -> Result<Query, SyntaxError> {
    let tokens = tokenize(src)?;
    parser::parse(tokens)
}
