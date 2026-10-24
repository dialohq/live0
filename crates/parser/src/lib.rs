use crate::{ast::Query, error::SyntaxError, lexer::tokenize};

mod ast;
mod error;
mod lexer;
mod parser;

pub fn parse(src: &str) -> Result<Query, SyntaxError> {
    let tokens = tokenize(src)?;
    parser::parse(tokens)
}
