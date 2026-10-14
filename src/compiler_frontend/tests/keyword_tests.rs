//! Compiler token mapping for the supported attached-bang keyword form.
//!
//! WHY: `return!` is the only word with an attached-bang token; every other spelling must stay
//! on the ordinary word path.

use crate::compiler_frontend::keywords::attached_bang_keyword_token_tag;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;

#[test]
fn return_bang_is_the_only_attached_keyword_token() {
    assert_eq!(
        attached_bang_keyword_token_tag("return"),
        Some(TokenTag::RETURN_BANG)
    );
    assert_eq!(attached_bang_keyword_token_tag("cast"), None);
    assert_eq!(attached_bang_keyword_token_tag("if"), None);
    assert_eq!(attached_bang_keyword_token_tag("return!"), None);
}
