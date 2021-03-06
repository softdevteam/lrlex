// Copyright (c) 2018 King's College London
// created by the Software Development Team <http://soft-dev.org/>
//
// The Universal Permissive License (UPL), Version 1.0
//
// Subject to the condition set forth below, permission is hereby granted to any person obtaining a
// copy of this software, associated documentation and/or data (collectively the "Software"), free
// of charge and under any and all copyright rights in the Software, and any and all patent rights
// owned or freely licensable by each licensor hereunder covering either (i) the unmodified
// Software as contributed to or provided by such licensor, or (ii) the Larger Works (as defined
// below), to deal in both
//
// (a) the Software, and
// (b) any piece of software and/or hardware listed in the lrgrwrks.txt file
// if one is included with the Software (each a "Larger Work" to which the Software is contributed
// by such licensors),
//
// without restriction, including without limitation the rights to copy, create derivative works
// of, display, perform, and distribute the Software and make, use, sell, offer for sale, import,
// export, have made, and have sold the Software and the Larger Work(s), and to sublicense the
// foregoing rights on either these or other terms.
//
// This license is subject to the following condition: The above copyright notice and either this
// complete permission notice or at a minimum a reference to the UPL must be included in all copies
// or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING
// BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
// NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
// DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

use std::collections::{HashMap, HashSet};
use std::convert::{AsRef, TryFrom};
use std::env::{current_dir, var};
use std::error::Error;
use std::fmt::Debug;
use std::fs::{File, read_to_string};
use std::io::Write;
use std::path::{Path, PathBuf};

use typename::TypeName;

use lexer::LexerDef;
use parser::parse_lex;

const LEX_SUFFIX: &str = "_l";
const LEX_FILE_EXT: &str = "l";
const RUST_FILE_EXT: &str = "rs";

/// Given the filename `x.l` as input, it will statically compile the file `src/x.l` into a Rust
/// module which can then be imported using `lrlex_mod!(x_l)`. This is a convenience function
/// around [`process_file`](fn.process_file.html) which makes it easier to compile `.l` files
/// stored in a project's `src/` directory. Note that leaf names must be unique within a single
/// project, even if they are in different directories: in other words, `a.l` and `x/a.l` will both
/// be mapped to the same module `a_l` (and it is undefined what the resulting Rust module will
/// contain).
///
/// See [`process_file`](fn.process_file.html)'s documentation for information about the
/// `rule_ids_map` argument and the returned tuple.
///
/// # Panics
///
/// If the input filename does not end in `.l`.
pub fn process_file_in_src<TokId>(srcp: &str,
                                  rule_ids_map: Option<HashMap<String, TokId>>)
                               -> Result<(Option<HashSet<String>>, Option<HashSet<String>>),
                                         Box<Error>>
                            where TokId: Copy + Debug + Eq + TryFrom<usize> + TypeName
{
    let mut inp = current_dir()?;
    inp.push("src");
    inp.push(srcp);
    if Path::new(srcp).extension().unwrap().to_str().unwrap() != LEX_FILE_EXT {
        panic!("File name passed to process_file_in_src must have extension '{}'.", LEX_FILE_EXT);
    }
    let mut leaf = inp.file_stem().unwrap().to_str().unwrap().to_owned();
    leaf.push_str(&LEX_SUFFIX);
    let mut outp = PathBuf::new();
    outp.push(var("OUT_DIR").unwrap());
    outp.push(leaf);
    outp.set_extension(RUST_FILE_EXT);
    process_file::<TokId, _, _>(inp, outp, rule_ids_map)
}

/// Statically compile the `.l` file `inp` into Rust, placing the output into `outp`. The latter
/// defines a module with a function `lexerdef()`, which returns a
/// [`LexerDef`](struct.LexerDef.html) that can then be used as normal.
///
/// If `None` is passed to `rule_ids_map` is ignored: lexing rules have arbitrary, but distinct,
/// IDs. If `Some(x)` is passed to `rule_ids_map` then the semantics of this parameter, and the
/// returned tuple are the same as [`set_rule_ids`](struct.LexerDef.html#method.set_rule_ids) (in
/// other words, `rule_ids_map` can be used to synchronise a lexer and parser, and to check that
/// all rules are used by both parts).
pub fn process_file<TokId, P, Q>(inp: P,
                                 outp: Q,
                                 rule_ids_map: Option<HashMap<String, TokId>>)
                              -> Result<(Option<HashSet<String>>, Option<HashSet<String>>),
                                        Box<Error>>
                           where TokId: Copy + Debug + Eq + TryFrom<usize> + TypeName,
                                 P: AsRef<Path>,
                                 Q: AsRef<Path>
{
    let inc = read_to_string(&inp).unwrap();
    let mut lexerdef = parse_lex::<TokId>(&inc)?;
    let (missing_from_lexer, missing_from_parser) = match rule_ids_map {
        Some(ref rim) => {
            // Convert from HashMap<String, _> to HashMap<&str, _>
            let owned_map = rim.iter()
                               .map(|(x, y)| (&**x, *y))
                               .collect::<HashMap<_, _>>();
            match lexerdef.set_rule_ids(&owned_map) {
                (x, y) => {
                    (x.map(|a| a.iter()
                                .map(|b| b.to_string())
                                .collect::<HashSet<_>>()),
                     y.map(|a| a.iter()
                                .map(|b| b.to_string())
                                .collect::<HashSet<_>>()))
                }
            }
        },
        None => (None, None)
    };

    let mut outs = String::new();
    let mod_name = inp.as_ref().file_stem().unwrap().to_str().unwrap();
    // Header
    outs.push_str(&format!("mod {}_l {{", mod_name));
    lexerdef.rust_pp(&mut outs);

    // Token IDs
    if let Some(rim) = rule_ids_map {
        for (n, id) in &rim {
            outs.push_str(&format!("#[allow(dead_code)]\nconst T_{}: {} = {:?};\n",
                                   n.to_ascii_uppercase(),
                                   TokId::type_name(),
                                   *id));
        }
    }

    // Footer
    outs.push_str("}");

    // If the file we're about to write out already exists with the same contents, then we don't
    // overwrite it (since that will force a recompile of the file, and relinking of the binary
    // etc).
    if let Ok(curs) = read_to_string(&outp) {
        if curs == outs {
            return Ok((missing_from_lexer, missing_from_parser));
        }
    }
    let mut f = File::create(outp)?;
    f.write_all(outs.as_bytes())?;
    Ok((missing_from_lexer, missing_from_parser))
}

impl<TokId: Copy + Debug + Eq + TypeName> LexerDef<TokId> {
    pub(crate) fn rust_pp(&self, outs: &mut String) {
        // Header
        outs.push_str(&format!("use lrlex::{{LexerDef, Rule}};

pub fn lexerdef() -> LexerDef<{}> {{
    let rules = vec![", TokId::type_name()));

        // Individual rules
        for r in &self.rules {
            let tok_id = match r.tok_id {
                Some(ref t) => format!("Some({:?})", t),
                None => "None".to_owned()
            };
            let n = match r.name {
                Some(ref n) => format!("Some({:?}.to_string())", n),
                None => "None".to_owned()
            };
            outs.push_str(&format!("
Rule::new({}, {}, \"{}\".to_string()).unwrap(),",
                tok_id, n, r.re_str.replace("\\", "\\\\").replace("\"", "\\\"")));
        }

        // Footer
        outs.push_str("
];
    LexerDef::new(rules)
}
");
    }
}
