# Grammar

## Status

This is the formal EBNF for Corvid v1.0, derived from the parser
implementation in `crates/corvid-syntax/src/parser/`. Two drift gates
in `crates/corvid-syntax/tests/grammar_drift.rs` keep it honest:

1. **Structural consistency** — every RHS reference resolves to a
   declared production or a listed terminal, and every production is
   reachable from `program`.
2. **Parse-evidence correspondence** — every production NOT marked
   `# PLANNED(<slice>)` must be mapped to a curated Corvid source
   snippet in the gate's evidence table, and that snippet must parse
   through the real parser. Adding a production without evidence (or
   with evidence that fails to parse) fails CI.

Productions marked `# PLANNED(<slice>)` are designed syntax that the
named ROADMAP slice implements — they are documentation of intent,
not descriptions of the shipped parser. When the slice ships, the
marker comes off and the evidence table gains a snippet.

The grammar is line-oriented: physical newlines are significant, with
continuation rules described in
[`docs/reference/lexer-rules.md`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/reference/lexer-rules.md).

## Notation

```
A ::= B C       — sequence
A ::= B | C     — alternative
A*              — zero or more
A+              — one or more
A?              — optional
'literal'       — literal token
INDENT, DEDENT  — virtual indentation tokens
NEWLINE         — physical newline (after continuation rules apply)
```

## Top level

```ebnf
program           ::= (decl | NEWLINE)* EOF

# `visibility` applies ONLY to type / store / tool / prompt / agent
# declarations (and annotation-prefixed agents). Effects, models,
# imports, extends, and the test surface cannot be `public` —
# `public effect …` / `public import …` are parse errors. Effect
# re-export is PLANNED(45o); import re-export is not scheduled.
decl              ::= visibility? (
                        type_decl
                      | store_decl
                      | tool_decl
                      | prompt_decl
                      | agent_decl
                      )
                    | import_decl
                    | server_decl
                    | schedule_decl
                    | eval_decl
                    | test_decl
                    | fixture_decl
                    | mock_decl
                    | extend_decl
                    | effect_decl
                    | model_decl

visibility        ::= 'public' ('(' 'package' ')')?
```

## Imports

Local modules, package URIs (`corvid://name@version`), remote URLs
(hash-pinned), and external-ecosystem imports all use string targets;
the `use` list is braceless with optional per-item aliases.

```ebnf
import_decl       ::= 'import' import_target ('as' IDENT)? ('use' import_list)? NEWLINE

import_target     ::= STRING_LITERAL             # "./module", "corvid://pkg@1.0.0", "https://…"
                    | 'python' STRING_LITERAL    # external ecosystem: import python "mylib"

import_list       ::= import_item (',' import_item)*

import_item       ::= IDENT ('as' IDENT)?
```

## Types and stores

```ebnf
type_decl         ::= 'type' IDENT ':' INDENT type_field+ DEDENT
                    | 'type' IDENT '=' type_alias_body NEWLINE      # PLANNED(45n)

type_field        ::= IDENT ':' type_ref NEWLINE
                    | '|' IDENT ('(' field_list ')')? NEWLINE       # PLANNED(45h) sum-type variant

field_list        ::= IDENT ':' type_ref (',' IDENT ':' type_ref)*  # PLANNED(45h)

type_alias_body   ::= type_ref                                      # PLANNED(45n)

store_decl        ::= ('session' | 'memory') IDENT ':' INDENT type_field+ DEDENT
```

## Type references

```ebnf
type_ref          ::= IDENT ('.' IDENT)? type_args?

type_args         ::= '<' type_arg (',' type_arg)* '>'

type_arg          ::= type_ref
                    | weak_effect_row     # only inside Weak<T, {...}>

weak_effect_row   ::= '{' weak_effect (',' weak_effect)* '}'

weak_effect       ::= 'tool_call' | 'llm' | 'approve' | 'human'   # builtin effect classes only
```

## Tool declarations

Tools are signature-only: the implementation is provided by the host
through registered-tool dispatch (executing stdlib, Rust FFI cdylib,
or Python host tools) — there is no tool body form.

```ebnf
tool_decl         ::= 'tool' IDENT params '->' type_ref ownership? 'dangerous'? uses_clause? NEWLINE

params            ::= '(' (param (',' param)*)? ')'

param             ::= IDENT ':' type_ref ownership?

ownership         ::= '@owned' | '@borrowed'

uses_clause       ::= 'uses' effect_name (',' effect_name)*

effect_name       ::= IDENT
```

## Prompt declarations

The prompt body is a single template string; parameters interpolate
with `{param}` inside the string (any typed parameter renders as its
JSON form). Role-block bodies are planned.

```ebnf
prompt_decl       ::= 'prompt' IDENT params '->' type_ref uses_clause? ':' INDENT prompt_body DEDENT

prompt_body       ::= role_clause* template_line

role_clause       ::= ('system' | 'user' | 'assistant') ':' STRING_LITERAL NEWLINE   # PLANNED(46b)

template_line     ::= STRING_LITERAL NEWLINE
```

Prompt bodies also accept structured clauses (`requires:`, `route:`,
`progressive:`, `rollout`, `ensemble`, `adversarial:`, `calibrated`,
`cacheable`, stream settings) before the template — documented in
the prompt chapters and the effect spec; their EBNF lands with a
future grammar-expansion pass rather than being half-specified here.

## Effect declarations

```ebnf
effect_decl       ::= 'effect' IDENT ':' INDENT dimension_assign+ DEDENT

dimension_assign  ::= IDENT ':' dimension_value NEWLINE

dimension_value   ::= 'true' | 'false'
                    | '$' NUMBER          # cost
                    | NUMBER              # confidence (0..1) or count
                    | IDENT               # named symbol like 'fast', 'grounded'
```

## Model declarations

```ebnf
model_decl        ::= 'model' IDENT ':' INDENT model_field+ DEDENT

model_field       ::= IDENT ':' dimension_value NEWLINE
```

## Agent declarations

```ebnf
agent_decl        ::= annotation* extern_abi? 'agent' IDENT params '->' type_ref uses_clause? ':' INDENT block DEDENT

# Annotation arguments are dimensional constraint values, not general
# expressions: `@budget($0.50)`, `@trust(autonomous)`, `@max_steps(10)`.
# Named arguments (`@idempotency(key: expr)`, `@retry(max_attempts: 3)`)
# are documented in earlier book drafts but DO NOT PARSE — filed with
# slice 45q alongside the keyword-collision fix for `@retry`.
annotation        ::= '@' IDENT ('(' annotation_args ')')?

annotation_args   ::= dimension_value

extern_abi        ::= 'pub' 'extern' STRING_LITERAL    # e.g. pub extern "c"

block             ::= stmt+

arg_list          ::= expr (',' expr)*
```

## Extension blocks

```ebnf
extend_decl       ::= 'extend' IDENT ':' INDENT extend_method+ DEDENT

extend_method     ::= visibility? (agent_decl | prompt_decl | tool_decl)
```

## Server and schedule declarations

Routes carry an HTTP method, an optional typed query/body contract,
a typed `json` response, and a handler body block. The `zone` clause
on schedules is mandatory.

```ebnf
server_decl       ::= 'server' IDENT ':' INDENT route_decl+ DEDENT

route_decl        ::= 'route' http_method STRING_LITERAL
                      ('query' type_ref)? ('body' type_ref)?
                      '->' 'json' type_ref uses_clause? ':' INDENT block DEDENT

http_method       ::= 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE'

schedule_decl     ::= 'schedule' STRING_LITERAL 'zone' STRING_LITERAL
                      '->' IDENT '(' arg_list? ')' uses_clause? NEWLINE
```

## Test surface

Fixtures and mocks are typed: both take a parameter list and a
return type; mocks name the tool/prompt they replace and may carry
an effect row. Tests may bind a recorded trace with `from_trace`.

```ebnf
eval_decl         ::= 'eval' IDENT ':' INDENT eval_body DEDENT

test_decl         ::= 'test' IDENT ('from_trace' STRING_LITERAL)? ':' INDENT eval_body DEDENT

fixture_decl      ::= 'fixture' IDENT params '->' type_ref ':' INDENT block DEDENT

mock_decl         ::= 'mock' IDENT params '->' type_ref uses_clause? ':' INDENT block DEDENT

eval_body         ::= (assertion | stmt)+

assertion         ::= 'assert' expr NEWLINE
                    | 'assert_snapshot' STRING_LITERAL NEWLINE
```

## Statements

```ebnf
stmt              ::= return_stmt
                    | yield_stmt
                    | if_stmt
                    | for_stmt
                    | approve_stmt
                    | break_stmt | continue_stmt | pass_stmt
                    | assign_stmt
                    | expr_stmt

return_stmt       ::= 'return' expr? NEWLINE

yield_stmt        ::= 'yield' expr NEWLINE

if_stmt           ::= 'if' expr ':' INDENT block DEDENT
                      ('else' ':' INDENT block DEDENT)?
                      # 'elif' chaining PLANNED(45q); 'while' loops PLANNED(45k)

for_stmt          ::= 'for' IDENT 'in' expr ':' INDENT block DEDENT

approve_stmt      ::= 'approve' IDENT '(' arg_list? ')' NEWLINE

break_stmt        ::= 'break' NEWLINE
continue_stmt     ::= 'continue' NEWLINE
pass_stmt         ::= 'pass' NEWLINE

# Bindings are bare (`x = expr`, type inferred) or annotated
# (`x: Int = expr` — the same `name: Type` shape fields and params
# use; the checker verifies initializer agreement). There is
# deliberately NO `let` keyword: one binding form, coherent with the
# Python-flavored surface (decision recorded at ROADMAP slice 45a).
#
# Place assignment (45b) writes through a path rooted at a local:
# `w.balance = v`, `xs[i] = v`, nested `acct.wallet.scores[0] = v`,
# and compound `+= -= *= /= %=`. Reference semantics: structs and
# lists are shared heap cells, so mutation through one binding is
# visible through every alias. The compound operator is NOT
# desugared — the place (including index expressions) evaluates
# exactly once.
assign_stmt       ::= IDENT (':' type_ref)? '=' expr NEWLINE
                    | place assign_op expr NEWLINE

place             ::= IDENT (('.' IDENT) | ('[' expr ']'))+
                    | IDENT                                  # compound only

assign_op         ::= '=' | '+=' | '-=' | '*=' | '/=' | '%='

expr_stmt         ::= expr NEWLINE
```

## Expressions

Pratt-style precedence climbing, lowest-to-highest:

```ebnf
expr              ::= or_expr

or_expr           ::= and_expr ('or' and_expr)*
and_expr          ::= not_expr ('and' not_expr)*
not_expr          ::= 'not' not_expr | cmp_expr
cmp_expr          ::= add_expr (cmp_op add_expr)?         # no chaining
cmp_op            ::= '==' | '!=' | '<' | '<=' | '>' | '>='
add_expr          ::= mul_expr (('+' | '-') mul_expr)*
mul_expr          ::= unary_expr (('*' | '/' | '%') unary_expr)*
# unary '+' PLANNED(45q)
unary_expr        ::= '-'? postfix_expr
postfix_expr      ::= primary_expr postfix_op*
postfix_op        ::= '(' arg_list? ')'                    # call
                    | '.' IDENT                            # field/method
                    | '[' expr ']'                         # index
                    | '?'                                  # try-propagate

primary_expr      ::= literal
                    | IDENT
                    | '(' expr ')'
                    | list_literal
                    | map_literal
                    | struct_literal                       # PLANNED(45n)
                    | match_expr                           # PLANNED(45i)
                    | retry_expr

literal           ::= INT | FLOAT | STRING | 'true' | 'false' | 'Nothing'

list_literal      ::= '[' (expr (',' expr)*)? ']'

# Duplicate keys in a literal: the LAST occurrence wins (Python).
# Trailing comma allowed. `{}` is the empty map.
map_literal       ::= '{' (map_entry (',' map_entry)* ','?)? '}'
map_entry         ::= expr ':' expr

struct_literal    ::= IDENT '{' field_init (',' field_init)* '}'     # PLANNED(45n)
field_init        ::= IDENT (':' expr)?                              # PLANNED(45n)
                    | '..' expr             # spread / update

match_expr        ::= 'match' expr ':' INDENT match_arm+ DEDENT      # PLANNED(45i)
match_arm         ::= pattern ('if' expr)? '->' expr NEWLINE         # PLANNED(45i)

pattern           ::= literal_pattern                                # PLANNED(45i)
                    | IDENT                                  # binding
                    | IDENT '(' pattern (',' pattern)* ')'   # sum-type variant
                    | IDENT '{' field_pattern (',' field_pattern)* (',' '..')? '}'
                    | '_'                                    # wildcard

literal_pattern   ::= literal                                        # PLANNED(45i)

field_pattern     ::= IDENT (':' pattern)?                           # PLANNED(45i)

retry_expr        ::= 'try' expr 'on' 'error' 'retry' INT 'times'
                      'backoff' ('linear' | 'exponential') INT    # base delay in ms; backoff is mandatory
```

## Lexical tokens

Keywords (reserved): `agent`, `tool`, `prompt`, `eval`, `test`,
`fixture`, `mock`, `server`, `route`, `schedule`, `zone`, `type`,
`session`, `memory`, `import`, `as`, `pub`, `extern`, `extend`,
`public`, `package`, `try`, `on`, `error`, `retry`, `times`,
`backoff`, `linear`, `exponential`, `approve`, `dangerous`, `effect`,
`uses`, `assert`, `assert_snapshot`, `model`, `requires`,
`progressive`, `below`, `rollout`, `ensemble`, `vote`, `adversarial`,
`propose`, `challenge`, `adjudicate`, `if`, `else`, `for`, `in`,
`return`, `yield`, `break`, `continue`, `pass`, `replay`, `when`,
`true`, `false`, `and`, `or`, `not`.

Contextual words (parsed positionally, NOT reserved — they are valid
identifiers elsewhere): `use` (import lists), `Nothing` (the unit
literal/type), `system`/`user`/`assistant` (planned role clauses),
`python` (import source). There is no `let` keyword: bindings are
bare or annotated assignment (decision recorded at ROADMAP slice
45a), so `let` remains an ordinary identifier permanently.

Identifiers: `[A-Za-z_][A-Za-z0-9_]*` excluding keywords.

Numeric literals: `INT` is `[0-9]+`; `FLOAT` is
`[0-9]+\.[0-9]+(e[+-]?[0-9]+)?`. There is no hex literal form.

String literals: `"..."` (single-line, escape-processed) or
`"""..."""` (multi-line, raw).

## Authoritative source

The parser at `crates/corvid-syntax/src/parser/` is the source of
truth. This grammar was extracted from it; if a future parser change
diverges, the drift gates fail and one of the two must change. The
per-production parse-evidence snippets live in
[`crates/corvid-syntax/tests/grammar_drift.rs`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/crates/corvid-syntax/tests/grammar_drift.rs).
