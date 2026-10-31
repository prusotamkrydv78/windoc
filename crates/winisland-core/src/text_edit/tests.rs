use super::*;

#[test]
fn typing_inserts_at_the_caret() {
    let mut buffer = TextBuffer::new(20);
    assert!(buffer.insert("helo"));
    buffer.apply(EditKey::Left);
    assert!(buffer.insert("l"));
    assert_eq!(buffer.text(), "hello");
    assert_eq!(buffer.caret(), 4);
    assert_eq!(buffer.before_caret(), "hell");
}

#[test]
fn backspace_and_delete_respect_edges() {
    let mut buffer = TextBuffer::with_text("abc", 20);
    assert!(!buffer.apply(EditKey::Delete));
    assert!(buffer.apply(EditKey::Backspace));
    assert_eq!(buffer.text(), "ab");
    buffer.apply(EditKey::Home);
    assert!(!buffer.apply(EditKey::Backspace));
    assert!(buffer.apply(EditKey::Delete));
    assert_eq!(buffer.text(), "b");
    assert_eq!(buffer.caret(), 0);
}

#[test]
fn multibyte_text_is_edited_by_character() {
    let mut buffer = TextBuffer::with_text("北京é", 20);
    assert_eq!(buffer.char_count(), 3);
    buffer.apply(EditKey::Left);
    assert!(buffer.apply(EditKey::Backspace));
    assert_eq!(buffer.text(), "北é");
    assert_eq!(buffer.before_caret(), "北");
    buffer.apply(EditKey::End);
    assert_eq!(buffer.caret(), 2);
}

#[test]
fn limit_and_control_characters_are_enforced() {
    let mut buffer = TextBuffer::new(5);
    assert!(buffer.insert("ab\ncd\tef"));
    assert_eq!(buffer.text(), "abcde");
    assert!(!buffer.insert("x"));
    assert!(!buffer.insert("\u{7}"));
    assert_eq!(TextBuffer::with_text("toolong", 3).text(), "too");
}

#[test]
fn navigation_reports_changes_only_when_moving() {
    let mut buffer = TextBuffer::with_text("ab", 10);
    assert!(!buffer.apply(EditKey::Right));
    assert!(!buffer.apply(EditKey::End));
    assert!(buffer.apply(EditKey::Home));
    assert!(!buffer.apply(EditKey::Left));
    buffer.set_caret(99);
    assert_eq!(buffer.caret(), 2);
    buffer.clear();
    assert!(buffer.is_empty());
    assert_eq!(buffer.caret(), 0);
}
