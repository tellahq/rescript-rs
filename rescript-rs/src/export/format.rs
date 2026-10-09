//! Parses the type declarations rescript-rs generates and prints them the way
//! `rescript format` does (`print_type_declaration2` and friends in the
//! compiler's `res_printer.ml`), so generated files never differ from their
//! formatted form.
//!
//! The formatter keeps some layout choices from its input (a record whose
//! first field is on a new line stays broken, attributes on their own line
//! stay there). The layouts chosen here are fixed points of it: records and
//! multi-constructor variants always break, and the first declaration's
//! attributes sit on their own line.

use super::doc::{
    breakable_group, concat, group, hard_line, if_breaks, indent, join, line, soft_line, text,
    to_string, trailing_comma, Doc,
};

pub(crate) struct Decl {
    docs: Vec<String>,
    attrs: Vec<String>,
    pub(crate) name: String,
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
    docs: Vec<String>,
    attrs: Vec<String>,
    name: String,
    optional: bool,
    ty: Ty,
}

struct Constructor {
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
}

#[derive(Clone, PartialEq, Debug)]
enum Token {
    Ident(String),
    TypeVar(String),
    DocComment(String),
    Attribute(String),
    Punct(char),
}

fn lex(src: &str) -> Option<Vec<Token>> {
    let bytes = src.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;

    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'\'';
    let string_end = |start: usize| -> Option<usize> {
        let mut j = start + 1;
        while j < bytes.len() {
            match bytes[j] {
                b'\\' => j += 2,
                b'"' => return Some(j + 1),
                _ => j += 1,
            }
        }
        None
    };

    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
        } else if src[i..].starts_with("/**") {
            let end = i + 3 + src[i + 3..].find("*/")?;
            tokens.push(Token::DocComment(src[i + 3..end].to_owned()));
            i = end + 2;
        } else if b == b'@' {
            let mut j = i + 1;
            while j < bytes.len() && (is_ident(bytes[j]) || bytes[j] == b'.') {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'(' {
                j = payload_end(bytes, j, &string_end)?;
            }
            tokens.push(Token::Attribute(src[i..j].to_owned()));
            i = j;
        } else if b == b'\\' && bytes.get(i + 1) == Some(&b'"') {
            let end = string_end(i + 1)?;
            tokens.push(Token::Ident(src[i..end].to_owned()));
            i = end;
        } else if b == b'\'' {
            let mut j = i + 1;
            while j < bytes.len() && is_ident(bytes[j]) {
                j += 1;
            }
            tokens.push(Token::TypeVar(src[i..j].to_owned()));
            i = j;
        } else if b.is_ascii_alphabetic() || b == b'_' {
            let mut j = i;
            while j < bytes.len() && is_ident(bytes[j]) {
                j += 1;
            }
            tokens.push(Token::Ident(src[i..j].to_owned()));
            i = j;
        } else if b"{}()<>,:?=|.".contains(&b) {
            tokens.push(Token::Punct(b as char));
            i += 1;
        } else {
            return None;
        }
    }

    Some(tokens)
}

/// The end of an attribute payload: a single string or number literal.
fn payload_end(
    bytes: &[u8],
    open: usize,
    string_end: &dyn Fn(usize) -> Option<usize>,
) -> Option<usize> {
    let close = match bytes.get(open + 1)? {
        b'"' => string_end(open + 1)?,
        b'0'..=b'9' | b'-' => {
            let mut j = open + 2;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            j
        }
        _ => return None,
    };
    (bytes.get(close) == Some(&b')')).then_some(close + 1)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn peek_at(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.pos + offset)
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.pos).cloned();
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

    fn docs_and_attrs(&mut self) -> (Vec<String>, Vec<String>) {
        let (mut docs, mut attrs) = (Vec::new(), Vec::new());
        loop {
            match self.peek() {
                Some(Token::DocComment(d)) => docs.push(d.clone()),
                Some(Token::Attribute(a)) => attrs.push(a.clone()),
                _ => return (docs, attrs),
            }
            self.pos += 1;
        }
    }

    fn decls(&mut self) -> Option<Vec<Decl>> {
        let mut decls = Vec::new();
        while self.peek().is_some() {
            decls.push(self.decl()?);
        }
        Some(decls)
    }

    fn decl(&mut self) -> Option<Decl> {
        let (docs, attrs) = self.docs_and_attrs();
        if self.eat_keyword("type") {
            self.eat_keyword("rec");
        } else if !self.eat_keyword("and") {
            return None;
        }
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
            docs,
            attrs,
            name,
            params,
            kind,
        })
    }

    fn starts_variant(&self) -> bool {
        match self.peek() {
            Some(Token::Punct('|') | Token::DocComment(_) | Token::Attribute(_)) => true,
            Some(Token::Ident(s)) => {
                s.starts_with(|c: char| c.is_ascii_uppercase())
                    && self.peek_at(1) != Some(&Token::Punct('.'))
            }
            _ => false,
        }
    }

    /// Fields up to and including the closing brace.
    fn fields(&mut self) -> Option<Vec<Field>> {
        let mut fields = Vec::new();
        while !self.eat('}') {
            let (docs, attrs) = self.docs_and_attrs();
            let name = self.ident()?;
            let optional = self.eat('?');
            self.expect(':')?;
            let ty = self.ty()?;
            fields.push(Field {
                docs,
                attrs,
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
            let (mut docs, mut attrs) = self.docs_and_attrs();
            let bar = self.eat('|');
            if !bar && !constructors.is_empty() {
                self.pos = start;
                return Some(constructors);
            }
            let (more_docs, more_attrs) = self.docs_and_attrs();
            docs.extend(more_docs);
            attrs.extend(more_attrs);
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
                docs,
                attrs,
                name,
                args,
            });
            if !matches!(
                self.peek(),
                Some(Token::Punct('|') | Token::DocComment(_) | Token::Attribute(_))
            ) {
                return Some(constructors);
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
        match self.next()? {
            Token::TypeVar(v) => Some(Ty::Var(v)),
            Token::Punct('{') => {
                let fields = self.fields()?;
                (!fields.is_empty()).then_some(Ty::Record(fields))
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

/// `@name(payload)` prints its non-huggable payload between soft lines.
fn print_attribute(attr: &str) -> Doc {
    match attr.split_once('(') {
        None => text(attr),
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

fn print_field(field: &Field) -> Doc {
    let line_break = if field.docs.is_empty() {
        line()
    } else {
        text(" ")
    };
    group(concat(vec![
        print_attributes(&field.docs, &field.attrs, line_break),
        text(&field.name),
        text(if field.optional { "?: " } else { ": " }),
        print_ty(&field.ty),
    ]))
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
        (0, 1) => "type ",
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
    let docs = decls
        .iter()
        .enumerate()
        .map(|(i, d)| print_decl(i, decls.len(), d))
        .collect();
    to_string(join(|| concat(vec![hard_line(), hard_line()]), docs))
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
}
