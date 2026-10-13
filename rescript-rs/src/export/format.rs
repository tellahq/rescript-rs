//! Parses the type declarations rescript-rs generates and prints them the way
//! `rescript format` does (`print_type_declaration2` and friends in the
//! compiler's `res_printer.ml`), so generated files never differ from their
//! formatted form.
//!
//! The formatter keeps some layout choices from its input (a record whose
//! first field is on a new line stays broken, attributes on their own line
//! stay there). The layouts chosen here are fixed points of it: records and
//! multi-constructor variants always break, the first declaration's
//! attributes sit on their own line, and a field's attributes and a
//! polymorphic variant keep the line breaks they were generated with.

use super::doc::{
    breakable_group, concat, group, hard_line, if_breaks, indent, join, line, soft_line, text,
    to_string, trailing_comma, Doc, WIDTH,
};

pub(crate) struct Decl {
    /// `//` lines, printed above everything else.
    comments: Vec<String>,
    docs: Vec<String>,
    attrs: Vec<String>,
    pub(crate) name: String,
    rec: bool,
    params: Vec<String>,
    kind: Kind,
}

enum Kind {
    Abstract,
    Alias(Ty),
    Record(Vec<Field>),
    Variant(Vec<Constructor>),
}

struct Field {
    comments: Vec<String>,
    docs: Vec<String>,
    attrs: Vec<String>,
    /// The attributes were on their own line.
    attrs_break: bool,
    name: String,
    optional: bool,
    ty: Ty,
}

struct Constructor {
    comments: Vec<String>,
    docs: Vec<String>,
    attrs: Vec<String>,
    name: String,
    args: Args,
}

enum Args {
    None,
    Tuple(Vec<Ty>),
    Record(Vec<Field>),
}

enum Ty {
    Var(String),
    Tuple(Vec<Ty>),
    Constr {
        path: String,
        args: Vec<Ty>,
    },
    /// An inline record definition; `{}` is an empty object type instead.
    Record(Vec<Field>),
    /// A closed polymorphic variant, `[#A | #B(int)]`.
    Poly {
        tags: Vec<Tag>,
        broken: bool,
    },
    Attributed(Vec<String>, Box<Ty>),
}

/// A polymorphic variant constructor.
struct Tag {
    comments: Vec<String>,
    attrs: Vec<String>,
    name: String,
    args: Vec<Ty>,
}

#[derive(Clone, PartialEq, Debug)]
enum Token {
    Ident(String),
    TypeVar(String),
    DocComment(String),
    LineComment(String),
    Attribute(String),
    Punct(char),
}

/// The tokens, each with whether a line break precedes it.
fn lex(src: &str) -> Option<Vec<(Token, bool)>> {
    let bytes = src.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    let mut newline = false;

    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'\'';

    while i < bytes.len() {
        let b = bytes[i];
        let token = if b.is_ascii_whitespace() {
            newline |= b == b'\n';
            i += 1;
            continue;
        } else if src[i..].starts_with("/**") {
            let end = i + 3 + src[i + 3..].find("*/")?;
            let token = Token::DocComment(src[i + 3..end].to_owned());
            i = end + 2;
            token
        } else if src[i..].starts_with("//") {
            let end = src[i..].find('\n').map_or(src.len(), |n| i + n);
            let token = Token::LineComment(src[i + 2..end].trim_end().to_owned());
            i = end;
            token
        } else if b == b'@' {
            let mut j = i + 1;
            while j < bytes.len() && (is_ident(bytes[j]) || bytes[j] == b'.') {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'(' {
                j = payload_end(src, j)?;
            }
            let token = Token::Attribute(src[i..j].to_owned());
            i = j;
            token
        } else if b == b'\\' && bytes.get(i + 1) == Some(&b'"') {
            let end = string_end(bytes, i + 1)?;
            let token = Token::Ident(src[i..end].to_owned());
            i = end;
            token
        } else if b == b'\'' {
            let mut j = i + 1;
            while j < bytes.len() && is_ident(bytes[j]) {
                j += 1;
            }
            let token = Token::TypeVar(src[i..j].to_owned());
            i = j;
            token
        } else if b.is_ascii_alphabetic() || b == b'_' {
            let mut j = i;
            while j < bytes.len() && is_ident(bytes[j]) {
                j += 1;
            }
            let token = Token::Ident(src[i..j].to_owned());
            i = j;
            token
        } else if b"{}()<>[],:?=|.#".contains(&b) {
            i += 1;
            Token::Punct(b as char)
        } else {
            return None;
        };

        tokens.push((token, newline));
        newline = false;
    }

    Some(tokens)
}

/// The index after the string literal whose opening quote is at `start`.
fn string_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut j = start + 1;
    while j < bytes.len() {
        match bytes[j] {
            b'\\' => j += 2,
            b'"' => return Some(j + 1),
            _ => j += 1,
        }
    }
    None
}

/// The end of an attribute payload that `rescript format` prints unchanged:
/// a string or number literal, a value path (`Story`, `Codecs.date`), `[]`,
/// or a one-line record of such values (`{legacy: true}`).
fn payload_end(src: &str, open: usize) -> Option<usize> {
    let bytes = src.as_bytes();
    let close = match bytes.get(open + 1)? {
        b'"' => string_end(bytes, open + 1)?,
        b'[' if bytes.get(open + 2) == Some(&b']') => open + 3,
        b'{' => open + 1 + src[open + 1..].find('}')? + 1,
        _ => {
            let mut j = open + 1;
            while j < bytes.len() && !matches!(bytes[j], b')' | b'(' | b'\n') {
                j += 1;
            }
            j
        }
    };
    let payload = &src[open + 1..close];

    let valid = match payload.as_bytes().first()? {
        b'"' => true,
        b'[' => true,
        b'{' => payload[1..payload.len() - 1].split(", ").all(
            |entry| matches!(entry.split_once(": "), Some((k, v)) if is_path(k) && is_atom(v)),
        ),
        _ => is_atom(payload),
    };

    (valid && bytes.get(close) == Some(&b')')).then_some(close + 1)
}

fn is_path(s: &str) -> bool {
    !s.is_empty()
        && s.split('.').all(|part| {
            part.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '\'')
        })
}

/// A number literal or a value path.
fn is_atom(s: &str) -> bool {
    let number = s.strip_prefix('-').unwrap_or(s);
    let is_number = number.starts_with(|c: char| c.is_ascii_digit())
        && number.parse::<f64>().is_ok()
        && !number.ends_with('.');

    is_number || is_path(s)
}

struct Parser {
    tokens: Vec<(Token, bool)>,
    pos: usize,
}

/// The comments, doc comments and attributes in front of an item.
struct Leading {
    comments: Vec<String>,
    docs: Vec<String>,
    attrs: Vec<String>,
    /// The item starts on a line after its last attribute.
    attrs_break: bool,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos).map(|(token, _)| token)
    }

    fn peek_at(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.pos + offset).map(|(token, _)| token)
    }

    /// Whether a line break precedes the next token.
    fn at_new_line(&self) -> bool {
        self.tokens
            .get(self.pos)
            .is_some_and(|(_, newline)| *newline)
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.peek().cloned();
        self.pos += 1;
        token
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(&Token::Punct(c)) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, c: char) -> Option<()> {
        self.eat(c).then_some(())
    }

    fn eat_keyword(&mut self, keyword: &str) -> bool {
        if matches!(self.peek(), Some(Token::Ident(s)) if s == keyword) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn ident(&mut self) -> Option<String> {
        match self.next()? {
            Token::Ident(s) => Some(s),
            _ => None,
        }
    }

    fn leading(&mut self) -> Leading {
        let mut leading = Leading {
            comments: Vec::new(),
            docs: Vec::new(),
            attrs: Vec::new(),
            attrs_break: false,
        };
        loop {
            match self.peek() {
                Some(Token::LineComment(c)) => leading.comments.push(c.clone()),
                Some(Token::DocComment(d)) => leading.docs.push(d.clone()),
                Some(Token::Attribute(a)) => leading.attrs.push(a.clone()),
                _ => {
                    leading.attrs_break = !leading.attrs.is_empty() && self.at_new_line();
                    return leading;
                }
            }
            self.pos += 1;
        }
    }

    fn starts_leading(&self) -> bool {
        matches!(
            self.peek(),
            Some(Token::LineComment(_) | Token::DocComment(_) | Token::Attribute(_))
        )
    }

    fn decls(&mut self) -> Option<Vec<Decl>> {
        let mut decls = Vec::new();
        while self.peek().is_some() {
            decls.push(self.decl()?);
        }
        Some(decls)
    }

    fn decl(&mut self) -> Option<Decl> {
        let Leading {
            comments,
            docs,
            attrs,
            ..
        } = self.leading();
        let rec = if self.eat_keyword("type") {
            self.eat_keyword("rec")
        } else if self.eat_keyword("and") {
            false
        } else {
            return None;
        };
        let name = self.ident()?;
        let mut params = Vec::new();
        if self.eat('<') {
            while !self.eat('>') {
                match self.next()? {
                    Token::TypeVar(v) => params.push(v),
                    _ => return None,
                }
                if !self.eat(',') {
                    self.expect('>')?;
                    break;
                }
            }
        }
        let kind = if !self.eat('=') {
            Kind::Abstract
        } else if self.eat('{') {
            Kind::Record(self.fields()?)
        } else if self.starts_variant() {
            Kind::Variant(self.constructors()?)
        } else {
            Kind::Alias(self.ty()?)
        };
        Some(Decl {
            comments,
            docs,
            attrs,
            name,
            rec,
            params,
            kind,
        })
    }

    fn starts_variant(&self) -> bool {
        match self.peek() {
            Some(Token::Ident(s)) => {
                s.starts_with(|c: char| c.is_ascii_uppercase())
                    && self.peek_at(1) != Some(&Token::Punct('.'))
            }
            _ => self.peeks('|') || self.starts_leading(),
        }
    }

    fn peeks(&self, c: char) -> bool {
        self.peek() == Some(&Token::Punct(c))
    }

    /// Fields up to and including the closing brace.
    fn fields(&mut self) -> Option<Vec<Field>> {
        let mut fields = Vec::new();
        while !self.eat('}') {
            let Leading {
                comments,
                docs,
                attrs,
                attrs_break,
            } = self.leading();
            let name = self.ident()?;
            let optional = self.eat('?');
            self.expect(':')?;
            let ty = self.ty()?;
            fields.push(Field {
                comments,
                docs,
                attrs,
                attrs_break,
                name,
                optional,
                ty,
            });
            if !self.eat(',') {
                self.expect('}')?;
                break;
            }
        }
        Some(fields)
    }

    fn constructors(&mut self) -> Option<Vec<Constructor>> {
        let mut constructors = Vec::new();
        loop {
            let start = self.pos;
            let mut leading = self.leading();
            let bar = self.eat('|');
            if !bar && !constructors.is_empty() {
                self.pos = start;
                return Some(constructors);
            }
            let more = self.leading();
            leading.comments.extend(more.comments);
            leading.docs.extend(more.docs);
            leading.attrs.extend(more.attrs);
            let name = match self.peek() {
                Some(Token::Ident(s)) if s.starts_with(|c: char| c.is_ascii_uppercase()) => {
                    self.ident()?
                }
                _ => return None,
            };
            let args = if !self.eat('(') {
                Args::None
            } else if self.eat('{') {
                let fields = self.fields()?;
                self.expect(')')?;
                Args::Record(fields)
            } else {
                Args::Tuple(self.ty_list(')')?)
            };
            constructors.push(Constructor {
                comments: leading.comments,
                docs: leading.docs,
                attrs: leading.attrs,
                name,
                args,
            });
            if !matches!(self.peek(), Some(Token::Punct('|'))) && !self.starts_leading() {
                return Some(constructors);
            }
        }
    }

    /// The tags of a polymorphic variant, after its `[`, up to and including
    /// the `]`.
    fn tags(&mut self) -> Option<Vec<Tag>> {
        let mut tags = Vec::new();
        loop {
            let mut leading = self.leading();
            if !self.eat('|') && !tags.is_empty() {
                return None;
            }
            let more = self.leading();
            if !leading.docs.is_empty() || !more.docs.is_empty() {
                return None;
            }
            leading.comments.extend(more.comments);
            leading.attrs.extend(more.attrs);
            self.expect('#')?;
            let name = self.ident()?;
            if !name.starts_with(|c: char| c.is_ascii_alphabetic()) {
                return None;
            }
            let args = if self.eat('(') {
                self.ty_list(')')?
            } else {
                Vec::new()
            };
            tags.push(Tag {
                comments: leading.comments,
                attrs: leading.attrs,
                name,
                args,
            });
            if self.eat(']') {
                return Some(tags);
            }
        }
    }

    /// Comma-separated types up to and including `close`.
    fn ty_list(&mut self, close: char) -> Option<Vec<Ty>> {
        let mut types = Vec::new();
        while !self.eat(close) {
            types.push(self.ty()?);
            if !self.eat(',') {
                self.expect(close)?;
                break;
            }
        }
        Some(types)
    }

    fn ty(&mut self) -> Option<Ty> {
        let start = self.pos;
        match self.next()? {
            Token::TypeVar(v) => Some(Ty::Var(v)),
            Token::Attribute(attr) => {
                let mut attrs = vec![attr];
                while let Some(Token::Attribute(a)) = self.peek() {
                    attrs.push(a.clone());
                    self.pos += 1;
                }
                Some(Ty::Attributed(attrs, Box::new(self.ty()?)))
            }
            Token::Punct('{') => {
                let fields = self.fields()?;
                (!fields.is_empty()).then_some(Ty::Record(fields))
            }
            Token::Punct('[') => {
                let tags = self.tags()?;
                let broken = self.tokens[start + 1..self.pos]
                    .iter()
                    .any(|(_, newline)| *newline);
                Some(Ty::Poly { tags, broken })
            }
            Token::Punct('(') => {
                let mut types = self.ty_list(')')?;
                match types.len() {
                    0 => None,
                    1 => types.pop(),
                    _ => Some(Ty::Tuple(types)),
                }
            }
            Token::Ident(first) => {
                let mut path = first;
                while self.eat('.') {
                    path.push('.');
                    path.push_str(&self.ident()?);
                }
                let args = if self.eat('<') {
                    self.ty_list('>')?
                } else {
                    Vec::new()
                };
                Some(Ty::Constr { path, args })
            }
            _ => None,
        }
    }
}

/// Parses generated declarations; `None` when they use syntax outside the
/// subset rescript-rs emits.
pub(crate) fn parse(src: &str) -> Option<Vec<Decl>> {
    let mut parser = Parser {
        tokens: lex(src)?,
        pos: 0,
    };
    parser.decls()
}

/// Leading `//` comments, each on its own line, as the formatter prints the
/// comments it attaches to the next node.
fn print_comments(comments: &[String]) -> Doc {
    let lines = comments
        .iter()
        .map(|c| concat(vec![text(format!("//{c}")), hard_line()]))
        .collect();
    concat(lines)
}

fn print_docs(docs: &[String]) -> Doc {
    if docs.is_empty() {
        return Doc::Nil;
    }
    let comments = docs.iter().map(|d| text(format!("/**{d}*/"))).collect();
    concat(vec![group(join(hard_line, comments)), hard_line()])
}

/// `print_attributes`: doc comments, then the other attributes followed by
/// `line_break`.
fn print_attributes(docs: &[String], attrs: &[String], line_break: Doc) -> Doc {
    let attrs = if attrs.is_empty() {
        Doc::Nil
    } else {
        let attrs = attrs.iter().map(|a| print_attribute(a)).collect();
        concat(vec![group(join(line, attrs)), line_break])
    };
    concat(vec![print_docs(docs), attrs])
}

/// `@name(payload)`: a record, array or string payload hugs the parentheses,
/// others sit between soft lines.
fn print_attribute(attr: &str) -> Doc {
    match attr.split_once('(') {
        None => text(attr),
        Some((name, payload)) if payload.starts_with(['{', '[', '"']) => {
            text(format!("{name}({payload}"))
        }
        Some((name, payload)) => group(concat(vec![
            text(name),
            text("("),
            indent(concat(vec![
                soft_line(),
                text(&payload[..payload.len() - 1]),
            ])),
            soft_line(),
            text(")"),
        ])),
    }
}

fn print_ty(ty: &Ty) -> Doc {
    match ty {
        Ty::Var(v) => text(v),
        Ty::Tuple(types) => group(print_tuple(types)),
        Ty::Record(fields) => group(print_fields(fields)),
        Ty::Poly { tags, broken } => print_poly(tags, *broken),
        Ty::Attributed(attrs, ty) => group(concat(vec![
            print_attributes(&[], attrs, line()),
            print_ty(ty),
        ])),
        Ty::Constr { path, args } => match args.as_slice() {
            [] => text(path),
            [Ty::Tuple(types)] => group(concat(vec![
                text(path),
                text("<"),
                print_tuple(types),
                text(">"),
            ])),
            args => group(concat(vec![
                text(path),
                text("<"),
                indent(concat(vec![
                    soft_line(),
                    comma_list(args.iter().map(print_ty).collect()),
                ])),
                trailing_comma(),
                soft_line(),
                text(">"),
            ])),
        },
    }
}

/// `Ptyp_variant`: a tag's leading comments follow its bar.
fn print_poly(tags: &[Tag], broken: bool) -> Doc {
    let rows = tags
        .iter()
        .enumerate()
        .map(|(i, tag)| {
            let bar = if i > 0 {
                text("| ")
            } else {
                if_breaks(text("| "), Doc::Nil)
            };
            let args = match tag.args.as_slice() {
                [] => Doc::Nil,
                [ty] => concat(vec![text("("), print_ty(ty), text(")")]),
                types => group(print_tuple(types)),
            };
            concat(vec![
                bar,
                print_comments(&tag.comments),
                group(concat(vec![
                    print_attributes(&[], &tag.attrs, line()),
                    text(format!("#{}", tag.name)),
                    args,
                ])),
            ])
        })
        .collect();

    breakable_group(
        broken,
        concat(vec![
            text("["),
            indent(concat(vec![soft_line(), join(line, rows)])),
            soft_line(),
            text("]"),
        ]),
    )
}

fn comma_list(docs: Vec<Doc>) -> Doc {
    join(|| concat(vec![text(","), line()]), docs)
}

fn print_tuple(types: &[Ty]) -> Doc {
    concat(vec![
        text("("),
        indent(concat(vec![
            soft_line(),
            comma_list(types.iter().map(print_ty).collect()),
        ])),
        trailing_comma(),
        soft_line(),
        text(")"),
    ])
}

/// A field's attributes stay on their own line when they were generated
/// there.
fn print_field(field: &Field) -> Doc {
    let line_break = if field.attrs_break {
        hard_line()
    } else if field.docs.is_empty() {
        line()
    } else {
        text(" ")
    };
    concat(vec![
        print_comments(&field.comments),
        group(concat(vec![
            print_attributes(&field.docs, &field.attrs, line_break),
            text(&field.name),
            text(if field.optional { "?: " } else { ": " }),
            print_ty(&field.ty),
        ])),
    ])
}

fn print_fields(fields: &[Field]) -> Doc {
    concat(vec![
        text("{"),
        indent(concat(vec![
            soft_line(),
            comma_list(fields.iter().map(print_field).collect()),
        ])),
        trailing_comma(),
        soft_line(),
        text("}"),
    ])
}

fn print_constructor(i: usize, constructor: &Constructor) -> Doc {
    let bar = if i > 0 || !constructor.docs.is_empty() || !constructor.attrs.is_empty() {
        text("| ")
    } else {
        if_breaks(text("| "), Doc::Nil)
    };
    let args = match &constructor.args {
        Args::None => Doc::Nil,
        Args::Tuple(types) => group(indent(print_tuple(types))),
        Args::Record(fields) => group(indent(concat(vec![
            text("("),
            print_fields(fields),
            text(")"),
        ]))),
    };
    concat(vec![
        print_comments(&constructor.comments),
        print_docs(&constructor.docs),
        bar,
        group(concat(vec![
            print_attributes(&[], &constructor.attrs, line()),
            text(&constructor.name),
            args,
        ])),
    ])
}

fn print_kind(kind: &Kind) -> Doc {
    match kind {
        Kind::Abstract => Doc::Nil,
        Kind::Alias(ty) => concat(vec![text(" = "), print_ty(ty)]),
        Kind::Record(fields) if fields.is_empty() => text(" = {}"),
        Kind::Record(fields) => concat(vec![
            text(" = "),
            breakable_group(true, print_fields(fields)),
        ]),
        Kind::Variant(constructors) => {
            let force_break = constructors.len() > 1;
            let rows = constructors
                .iter()
                .enumerate()
                .map(|(i, c)| print_constructor(i, c))
                .collect();
            concat(vec![
                text(" ="),
                breakable_group(
                    force_break,
                    indent(concat(vec![
                        line(),
                        breakable_group(force_break, join(line, rows)),
                    ])),
                ),
            ])
        }
    }
}

/// The first declaration of a `type rec` chain carries its doc comment on the
/// structure item, so its attributes keep their own line; later ones put
/// attributes after a doc comment on the `and` line.
fn print_decl(i: usize, count: usize, decl: &Decl) -> Doc {
    let (docs, attrs) = if i == 0 {
        (
            print_docs(&decl.docs),
            print_attributes(&[], &decl.attrs, hard_line()),
        )
    } else {
        let line_break = if decl.docs.is_empty() {
            line()
        } else {
            text(" ")
        };
        (
            Doc::Nil,
            print_attributes(&decl.docs, &decl.attrs, line_break),
        )
    };
    let prefix = match (i, count) {
        (0, 1) if !decl.rec => "type ",
        (0, _) => "type rec ",
        _ => "and ",
    };
    let params = if decl.params.is_empty() {
        Doc::Nil
    } else {
        group(concat(vec![
            text("<"),
            indent(concat(vec![
                soft_line(),
                comma_list(decl.params.iter().map(text).collect()),
            ])),
            trailing_comma(),
            soft_line(),
            text(">"),
        ]))
    };
    concat(vec![
        print_comments(&decl.comments),
        docs,
        group(concat(vec![
            attrs,
            text(prefix),
            text(&decl.name),
            params,
            print_kind(&decl.kind),
        ])),
    ])
}

/// The declarations as one formatted (`type rec … and …` when several) chain.
pub(crate) fn print(decls: &[&Decl]) -> String {
    print_in(decls, WIDTH)
}

/// [`print`] in `width` columns, for declarations nested in a module.
fn print_in(decls: &[&Decl], width: usize) -> String {
    let docs = decls
        .iter()
        .enumerate()
        .map(|(i, d)| print_decl(i, decls.len(), d))
        .collect();
    to_string(join(|| concat(vec![hard_line(), hard_line()]), docs), width)
}

#[cfg(feature = "spice")]
/// `source` laid out as `rescript format` would at `depth` levels of module
/// nesting, or unchanged when it uses syntax outside the subset rescript-rs
/// emits.
pub(crate) fn format(source: &str, depth: usize) -> String {
    match parse(source) {
        Some(decls) if !decls.is_empty() => {
            print_in(&decls.iter().collect::<Vec<_>>(), WIDTH - 2 * depth)
        }
        _ => source.to_owned(),
    }
}

// Expected outputs are fixed points of `rescript format` 12.3: formatting
// them changes nothing.
#[cfg(test)]
mod tests {
    use super::*;

    fn format(generated: &str) -> String {
        let decls = parse(generated).expect("parses");
        print(&decls.iter().collect::<Vec<_>>())
    }

    #[test]
    fn lays_out_a_declaration_chain_as_rescript_format_does() {
        let generated = r#"/**
 * A tagged union.
 */
@tag("type")
type background = 
  | Solid({ r: int, g: int, b: int, a: float, })
  | ShaderGradient({ shape: shaderGradientShape, color1: color, color2: color, speed: float, density: float, strength: float, })
  | Off({  })

/**
 * Doc and attribute on a later declaration.
 */
@tag("type")
type border = 
  | None({  })
  | Line({ width: float, })

type layoutName = 
  | @as("landscape-bottom-left-full-small-combi") LandscapeBottomLeftFullSmallCombi
  | @as("landscape-bottom-left-full-small-combi-letterbox") LandscapeBottomLeftFullSmallCombiLetterbox
  /**
   * Constructor doc.
   */
  | @as("x") X

type mouseEvent = {
  timestamp: int,
  
/**
 * Field doc.
 */
cursor: option<string>,
  @as("button_pressed") buttonPressed?: option<string>,
  overshoot: option<(float, float)>,
}

type pair<'a, 'b> = ('a, 'b)

@unboxed
type single = Single(Js.Dict.t<string>)

type empty = {  }
"#;

        assert_eq!(
            format(generated) + "\n",
            r#"/**
 * A tagged union.
 */
@tag("type")
type rec background =
  | Solid({r: int, g: int, b: int, a: float})
  | ShaderGradient({
      shape: shaderGradientShape,
      color1: color,
      color2: color,
      speed: float,
      density: float,
      strength: float,
    })
  | Off({})

/**
 * Doc and attribute on a later declaration.
 */
@tag("type") and border =
  | None({})
  | Line({width: float})

and layoutName =
  | @as("landscape-bottom-left-full-small-combi") LandscapeBottomLeftFullSmallCombi
  | @as("landscape-bottom-left-full-small-combi-letterbox")
  LandscapeBottomLeftFullSmallCombiLetterbox
  /**
   * Constructor doc.
   */
  | @as("x") X

and mouseEvent = {
  timestamp: int,
  /**
 * Field doc.
 */
  cursor: option<string>,
  @as("button_pressed") buttonPressed?: option<string>,
  overshoot: option<(float, float)>,
}

and pair<'a, 'b> = ('a, 'b)

@unboxed and single = Single(Js.Dict.t<string>)

and empty = {}
"#
        );
    }

    #[test]
    fn breaks_inline_records_only_past_the_line_width() {
        let generated = "type range = {\n  inner: { start: int, end: int, },\n  long: { aaaaaaaaaaaaaaaaaaaa: int, bbbbbbbbbbbbbbbbbbbbbbbb: int, cccccccccccccccccccccccc: int, dddd: int, },\n}";

        assert_eq!(
            format(generated),
            "type range = {\n  inner: {start: int, end: int},\n  long: {\n    aaaaaaaaaaaaaaaaaaaa: int,\n    bbbbbbbbbbbbbbbbbbbbbbbb: int,\n    cccccccccccccccccccccccc: int,\n    dddd: int,\n  },\n}"
        );
    }

    #[test]
    fn leaves_syntax_outside_the_generated_subset_unparsed() {
        assert!(parse("type t = { a: int } & other").is_none());
        assert!(parse("type t = { inner: {  } }").is_none());
    }

    #[cfg(feature = "spice")]
    #[test]
    fn lays_out_codec_declarations_as_rescript_format_does() {
        let row = "// A row.\n@spice\ntype t = {\n  // The key.\n  @spice.key(\"PK\")\n  pk: string,\n  @spice.default([])\n  words: array<string>,\n  @spice.key(\"a_long_json_key\") @spice.default(-1.5)\n  scale: float,\n  values: array<(int, int, int, int, int, int, int, int, int, int, int, int, int, int, int, string)>,\n  date: @spice.codec(Codecs.date) string,\n}";
        let poly = "// Poly.\n@spice.serde({legacy: true})\ntype rec t = [\n  // First.\n  | #A\n  | @spice.as(\"b\") #B(int, string)\n  | #C(array<t>)\n]";

        assert_eq!(
            super::format(row, 1),
            "// A row.\n@spice\ntype t = {\n  // The key.\n  @spice.key(\"PK\")\n  pk: string,\n  @spice.default([])\n  words: array<string>,\n  @spice.key(\"a_long_json_key\") @spice.default(-1.5)\n  scale: float,\n  values: array<(\n    int,\n    int,\n    int,\n    int,\n    int,\n    int,\n    int,\n    int,\n    int,\n    int,\n    int,\n    int,\n    int,\n    int,\n    int,\n    string,\n  )>,\n  date: @spice.codec(Codecs.date) string,\n}"
        );
        assert_eq!(
            super::format(poly, 0),
            "// Poly.\n@spice.serde({legacy: true})\ntype rec t = [\n  | // First.\n  #A\n  | @spice.as(\"b\") #B(int, string)\n  | #C(array<t>)\n]"
        );
    }
}
