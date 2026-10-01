//! A port of the ReScript formatter's layout engine (`res_doc.ml` in the
//! compiler), reduced to the documents type declarations use. Keeping its
//! semantics exact (forced-break propagation, the `fits` look-ahead over the
//! rest of the stack, byte widths, trailing-space trimming) is what makes the
//! generated files byte-identical to `rescript format` output.

const WIDTH: i64 = 100;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Line {
    /// A space when flat.
    Classic,
    /// Nothing when flat.
    Soft,
    /// Always a newline; breaks every enclosing group.
    Hard,
}

pub(super) enum Doc {
    Nil,
    Text(String),
    Concat(Vec<Doc>),
    Indent(Box<Doc>),
    IfBreaks {
        yes: Box<Doc>,
        no: Box<Doc>,
        broken: bool,
    },
    Line(Line),
    Group {
        should_break: bool,
        doc: Box<Doc>,
    },
}

pub(super) fn text(s: impl Into<String>) -> Doc {
    Doc::Text(s.into())
}

pub(super) fn concat(docs: Vec<Doc>) -> Doc {
    Doc::Concat(docs)
}

pub(super) fn indent(doc: Doc) -> Doc {
    Doc::Indent(Box::new(doc))
}

pub(super) fn group(doc: Doc) -> Doc {
    breakable_group(false, doc)
}

pub(super) fn breakable_group(force_break: bool, doc: Doc) -> Doc {
    Doc::Group {
        should_break: force_break,
        doc: Box::new(doc),
    }
}

pub(super) fn if_breaks(yes: Doc, no: Doc) -> Doc {
    Doc::IfBreaks {
        yes: Box::new(yes),
        no: Box::new(no),
        broken: false,
    }
}

pub(super) fn line() -> Doc {
    Doc::Line(Line::Classic)
}

pub(super) fn soft_line() -> Doc {
    Doc::Line(Line::Soft)
}

pub(super) fn hard_line() -> Doc {
    Doc::Line(Line::Hard)
}

pub(super) fn trailing_comma() -> Doc {
    if_breaks(text(","), Doc::Nil)
}

pub(super) fn join(sep: impl Fn() -> Doc, docs: Vec<Doc>) -> Doc {
    let mut out = Vec::with_capacity(docs.len() * 2);
    for (i, doc) in docs.into_iter().enumerate() {
        if i > 0 {
            out.push(sep());
        }
        out.push(doc);
    }
    concat(out)
}

fn propagate_forced_breaks(doc: &mut Doc) -> bool {
    match doc {
        Doc::Nil | Doc::Text(_) => false,
        Doc::Line(style) => *style == Line::Hard,
        Doc::Indent(child) => propagate_forced_breaks(child),
        Doc::IfBreaks { yes, no, broken } => {
            if propagate_forced_breaks(no) {
                propagate_forced_breaks(yes);
                *broken = true;
                true
            } else {
                propagate_forced_breaks(yes)
            }
        }
        Doc::Group { should_break, doc } => {
            let child = propagate_forced_breaks(doc);
            *should_break = *should_break || child;
            *should_break
        }
        Doc::Concat(children) => {
            // Every child must be walked, so no short-circuiting `any`.
            let mut forced = false;
            for child in children {
                forced |= propagate_forced_breaks(child);
            }
            forced
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Break,
    Flat,
}

type Command<'a> = (i64, Mode, &'a Doc);

struct Fits {
    width: i64,
    result: Option<bool>,
}

impl Fits {
    fn calculate(&mut self, mode: Mode, doc: &Doc) {
        if self.result.is_some() {
            return;
        }
        if self.width < 0 {
            self.result = Some(false);
            return;
        }
        match doc {
            Doc::Nil => {}
            Doc::Text(txt) => self.width -= txt.len() as i64,
            Doc::Indent(doc) => self.calculate(mode, doc),
            Doc::Line(style) => match (mode, style) {
                (Mode::Flat, Line::Hard) | (Mode::Break, _) => self.result = Some(true),
                (Mode::Flat, Line::Classic) => self.width -= 1,
                (Mode::Flat, Line::Soft) => {}
            },
            Doc::Group { should_break, doc } => {
                let mode = if *should_break { Mode::Break } else { mode };
                self.calculate(mode, doc)
            }
            Doc::IfBreaks { yes, no, broken } => {
                if *broken || mode == Mode::Break {
                    self.calculate(mode, yes)
                } else {
                    self.calculate(mode, no)
                }
            }
            Doc::Concat(docs) => {
                for doc in docs {
                    if self.result.is_some() {
                        return;
                    }
                    self.calculate(mode, doc);
                }
            }
        }
    }
}

/// Whether `next`, followed by the pending `rest` (top of the stack last),
/// fits in `width` columns up to the next line break.
fn fits(width: i64, next: Command, rest: &[Command]) -> bool {
    let mut fits = Fits {
        width,
        result: None,
    };
    for (_, mode, doc) in std::iter::once(next).chain(rest.iter().rev().copied()) {
        if let Some(result) = fits.result {
            return result;
        }
        fits.calculate(mode, doc);
    }
    fits.result.unwrap_or(fits.width >= 0)
}

fn flush_newline(out: &mut String) {
    let trimmed = out.trim_end_matches(' ').len();
    out.truncate(trimmed);
    out.push('\n');
}

pub(super) fn to_string(mut doc: Doc) -> String {
    propagate_forced_breaks(&mut doc);

    let mut out = String::new();
    let mut pos: i64 = 0;
    let mut stack: Vec<Command> = vec![(0, Mode::Flat, &doc)];

    while let Some((ind, mode, doc)) = stack.pop() {
        match doc {
            Doc::Nil => {}
            Doc::Text(txt) => {
                out.push_str(txt);
                pos += txt.len() as i64;
            }
            Doc::Concat(docs) => stack.extend(docs.iter().rev().map(|doc| (ind, mode, doc))),
            Doc::Indent(doc) => stack.push((ind + 2, mode, doc)),
            Doc::IfBreaks { yes, no, broken } => {
                let doc = if *broken || mode == Mode::Break {
                    yes
                } else {
                    no
                };
                stack.push((ind, mode, doc));
            }
            Doc::Line(style) => match (mode, style) {
                (Mode::Break, _) | (Mode::Flat, Line::Hard) => {
                    flush_newline(&mut out);
                    let ind = if mode == Mode::Break { ind } else { 0 };
                    out.extend(std::iter::repeat(' ').take(ind as usize));
                    pos = ind;
                }
                (Mode::Flat, Line::Classic) => {
                    out.push(' ');
                    pos += 1;
                }
                (Mode::Flat, Line::Soft) => {}
            },
            Doc::Group { should_break, doc } => {
                let flat = !should_break && fits(WIDTH - pos, (ind, Mode::Flat, doc), &stack);
                stack.push((ind, if flat { Mode::Flat } else { Mode::Break }, doc));
            }
        }
    }

    out
}
