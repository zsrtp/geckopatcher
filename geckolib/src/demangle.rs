use std::borrow::Cow;
use std::fmt::{self, Display};

type IsConst = bool;

#[derive(Debug, PartialEq, Eq, Clone)]
enum Type<'a> {
    Path(IsConst, Vec<&'a str>),
    Normal(IsConst, &'a str),
    Function(IsConst, Box<Type<'a>>, Vec<Type<'a>>),
    Array(usize, Box<Type<'a>>),
    Pointer(IsConst, Box<Type<'a>>),
    Reference(IsConst, Box<Type<'a>>),
    VarArgs,
}

struct DisplayLeft<'a>(&'a Type<'a>);
struct DisplayRight<'a>(&'a Type<'a>);

impl<'a> Display for DisplayLeft<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::Type::*;
        let &DisplayLeft(typ) = self;
        match *typ {
            Path(is_const, ref p) => {
                for (i, element) in p.iter().enumerate() {
                    if i != 0 {
                        write!(f, "::")?;
                    }
                    write!(f, "{}", element)?;
                }
                if is_const {
                    write!(f, " const")?;
                }
                Ok(())
            }
            Normal(is_const, t) => {
                write!(f, "{}", t)?;
                if is_const {
                    write!(f, " const")?;
                }
                Ok(())
            }
            Function(_, ref r, _) => write!(f, "{} (", DisplayLeft(r)),
            Array(_, ref t) => write!(f, "{} (", DisplayLeft(t)),
            Pointer(is_const, ref t) => {
                write!(f, "{}*", DisplayLeft(t))?;
                if is_const {
                    write!(f, " const")?;
                }
                Ok(())
            }
            Reference(is_const, ref t) => {
                write!(f, "{}&", DisplayLeft(t))?;
                if is_const {
                    write!(f, " const")?;
                }
                Ok(())
            }
            VarArgs => write!(f, "..."),
        }
    }
}

impl<'a> Display for DisplayRight<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::Type::*;
        let &DisplayRight(typ) = self;
        match *typ {
            Path(_, _) | Normal(_, _) | VarArgs => Ok(()),
            Function(is_const, ref r, ref p) => {
                write!(f, ")(")?;

                for (i, param) in p.iter().enumerate() {
                    if i != 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", param)?;
                }

                if is_const {
                    write!(f, ") const{}", DisplayRight(r))
                } else {
                    write!(f, "){}", DisplayRight(r))
                }
            }
            Array(count, ref t) => write!(f, ")[{}]{}", count, DisplayRight(t)),
            Pointer(_, ref t) => write!(f, "{}", DisplayRight(t)),
            Reference(_, ref t) => write!(f, "{}", DisplayRight(t)),
        }
    }
}

impl<'a> Display for Type<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}{}", DisplayLeft(self), DisplayRight(self))
    }
}

fn parse_count(text: &str) -> Result<(usize, &str), Cow<'static, str>> {
    let mut text = text;

    let count = text
        .chars()
        .take_while(|c| c.is_digit(10))
        .collect::<String>();
    text = &text[count.len()..];
    let count = count.parse().map_err(|_| "Couldn't parse count")?;

    Ok((count, text))
}

fn parse_type(text: &str) -> Result<(Option<Type>, &str), Cow<'static, str>> {
    let original_text = text;
    let mut text = text;
    let first_char = text.chars().next();
    if first_char.is_some() {
        text = &text[1..];
    }

    let typ = match first_char {
        Some('v') => Type::Normal(false, "void"),
        Some('c') => Type::Normal(false, "i8"),
        Some('s') => Type::Normal(false, "i16"),
        Some('i') | Some('l') => Type::Normal(false, "i32"),
        Some('x') => Type::Normal(false, "i64"),
        Some('f') => Type::Normal(false, "f32"),
        Some('d') => Type::Normal(false, "f64"),
        Some('b') => Type::Normal(false, "bool"),
        Some('e') => Type::VarArgs,
        Some('0'..='9') => {
            let (count, remaining) = parse_count(original_text)?;
            text = remaining;

            let type_name = &text[..count];
            text = &text[count..];

            Type::Normal(false, type_name)
        }
        Some('F') => {
            let mut parameters = Vec::new();

            while !text.starts_with('_') {
                let (typ, remaining) = parse_type(text)?;
                text = remaining;
                if let Some(typ) = typ {
                    parameters.push(typ);
                } else {
                    break;
                }
            }

            let typ = if !text.is_empty() {
                text = &text[1..];
                let (typ, remaining) = parse_type(text)?;
                text = remaining;

                typ
            } else {
                None
            };

            Type::Function(
                false,
                Box::new(typ.unwrap_or_else(|| Type::Normal(false, "void"))),
                parameters,
            )
        }
        Some('A') => {
            let (count, remaining) = parse_count(text)?;
            text = remaining;

            text = &text[1..];

            let (typ, remaining) = parse_type(text)?;
            text = remaining;

            Type::Array(
                count,
                Box::new(typ.ok_or("Expected Type of Array Elements")?),
            )
        }
        Some('P') => {
            let (typ, remaining) = parse_type(text)?;
            text = remaining;

            Type::Pointer(false, Box::new(typ.ok_or("Expected Type of Pointer")?))
        }
        Some('R') => {
            let (typ, remaining) = parse_type(text)?;
            text = remaining;

            Type::Reference(false, Box::new(typ.ok_or("Expected Type of Reference")?))
        }
        Some('Q') => {
            let skip_index = text.char_indices().nth(1).unwrap().0;
            let (count, _) = parse_count(&text[..skip_index])?;
            text = &text[skip_index..];

            let mut elements = Vec::with_capacity(count);

            for _ in 0..count {
                let (typ, remaining) = parse_type(text)?;
                text = remaining;
                let typ = typ.ok_or("Expected Path Element")?;

                if let Type::Normal(_, element) = typ {
                    elements.push(element);
                } else {
                    return Err(format!("Unexpected Path Element {}", typ).into());
                }
            }

            Type::Path(false, elements)
        }
        Some('U') => {
            let first_char = text.chars().next();
            let first_char = first_char.ok_or("Expected some unsigned type")?;
            text = &text[1..];

            match first_char {
                'c' => Type::Normal(false, "u8"),
                's' => Type::Normal(false, "u16"),
                'i' | 'l' => Type::Normal(false, "u32"),
                'x' => Type::Normal(false, "u64"),
                c => return Err(format!("Unexpected unsigned type {}", c).into()),
            }
        }
        Some('C') => {
            let (typ, remaining) = parse_type(text)?;
            text = remaining;
            let typ = typ.ok_or("Expected Constant Type")?;

            match typ {
                Type::Normal(_, name) => Type::Normal(true, name),
                Type::Path(_, elements) => Type::Path(true, elements),
                Type::Function(_, return_type, params) => Type::Function(true, return_type, params),
                Type::Pointer(_, typ) => Type::Pointer(true, typ),
                Type::Reference(_, typ) => Type::Reference(true, typ),
                t => return Err(format!("Unexpected const type {}", t).into()),
            }
        }
        Some('S') => return parse_type(text),
        Some(c) => return Err(format!("Unexpected token {}", c).into()),
        None => return Ok((None, text)),
    };

    Ok((Some(typ), text))
}

fn base_name(function: &str) -> (&str, Option<&str>) {
    if let Some(underscore_index) = function.rfind("__") {
        (
            &function[..underscore_index],
            Some(&function[underscore_index + 2..]),
        )
    } else {
        (function, None)
    }
}

pub fn demangle(function: &str) -> Result<Cow<str>, Cow<'static, str>> {
    fn extend_by_params(signature: &mut String, typ: Type) -> Result<(), Cow<'static, str>> {
        if let Type::Function(is_const, return_value, params) = typ {
            signature.push_str("(");
            for (i, param) in params.iter().enumerate() {
                if param == &Type::Normal(false, "void") {
                    continue;
                }
                if i != 0 {
                    signature.push_str(", ");
                }
                signature.push_str(&param.to_string());
            }
            signature.push_str(")");

            if return_value.as_ref() != &Type::Normal(false, "void") {
                signature.push_str(" -> ");
                signature.push_str(&return_value.to_string());
            }

            if is_const {
                signature.push_str(" const");
            }

            Ok(())
        } else {
            Err(format!(
                "Unexpected Type {} for signature {}, expected Function",
                typ, signature
            ).into())
        }
    }

    fn extend_by_base_name(signature: &mut String, base_name: &str) {
        if base_name == "__ct" {
        } else if base_name == "__dt" {
            *signature = format!("~{}", signature);
        } else if !base_name.is_empty() {
            if !signature.is_empty() {
                signature.push_str("::");
            }
            signature.push_str(base_name);
        }
    }

    let (base_name, path) = base_name(function);

    if base_name.is_empty() {
        return Ok(function.into());
    }

    if let Some(mut text) = path {
        let mut result = String::new();

        let (typ, remaining) = parse_type(text)?;
        text = remaining;

        let typ = typ.ok_or("Expected path")?;

        match typ {
            Type::Path(_, path) => {
                for (i, element) in path.iter().enumerate() {
                    if i != 0 {
                        result.push_str("::");
                    }
                    result.push_str(element);
                }
                extend_by_base_name(&mut result, base_name);

                let (typ, _) = parse_type(text)?;
                if let Some(typ) = typ {
                    extend_by_params(&mut result, typ)?;
                }
            }
            Type::Normal(_, name) => {
                result.push_str(name);
                extend_by_base_name(&mut result, base_name);

                let (typ, _) = parse_type(text)?;
                if let Some(typ) = typ {
                    extend_by_params(&mut result, typ)?;
                }
            }
            t => {
                extend_by_base_name(&mut result, base_name);
                extend_by_params(&mut result, t)?;
            }
        }

        Ok(result.into())
    } else {
        Ok(base_name.into())
    }
}
