# Grammar

## Status

This is the formal EBNF for Corvid v1.0, derived from the parser
implementation in `crates/corvid-syntax/src/parser/`. A drift-gate
test (slice 33J6) cross-checks every production listed here against
the parser's tests. The grammar is line-oriented: physical newlines
are significant, with continuation rules described in
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

decl              ::= visibility? (
                        import_decl
                      | type_decl
                      | store_decl
                      | tool_decl
                      | prompt_decl
                      | server_decl
                      | schedule_decl
                      | eval_decl
                      | test_decl
                      | fixture_decl
                      | mock_decl
                      | agent_decl
                      | extend_decl
                      | effect_decl
                      | model_decl
                      )

visibility        ::= 'public' ('(' 'package' ')')?
```

## Imports

```ebnf
import_decl       ::= 'import' import_target ('as' IDENT)? ('use' '{' import_list '}')? NEWLINE

import_target     ::= IDENT             # local module
                    | STRING_LITERAL    # package: "@scope/name"

import_list       ::= IDENT (',' IDENT)*
```

## Types and stores

```ebnf
type_decl         ::= 'type' IDENT (':' INDENT type_field+ DEDENT | '=' type_alias_body NEWLINE)

type_field        ::= IDENT ':' type_ref NEWLINE
                    | '|' IDENT ('(' field_list ')')? NEWLINE   # sum-type variant

field_list        ::= IDENT ':' type_ref (',' IDENT ':' type_ref)*

type_alias_body   ::= type_ref

store_decl        ::= ('session' | 'memory') IDENT ':' INDENT type_field+ DEDENT
```

## Type references

```ebnf
type_ref          ::= IDENT ('.' IDENT)? type_args?

type_args         ::= '<' type_arg (',' type_arg)* '>'

type_arg          ::= type_ref
                    | weak_effect_row     # only inside Weak<T, {...}>

weak_effect_row   ::= '{' weak_effect (',' weak_effect)* '}'

weak_effect       ::= IDENT
```

## Tool declarations

```ebnf
tool_decl         ::= 'tool' IDENT params '->' type_ref ownership? 'dangerous'? uses_clause? NEWLINE

params            ::= '(' (param (',' param)*)? ')'

param             ::= IDENT ':' type_ref ownership?

ownership         ::= '@owned' | '@borrowed'

uses_clause       ::= 'uses' effect_name (',' effect_name)*

effect_name       ::= IDENT
```

## Prompt declarations

```ebnf
prompt_decl       ::= 'prompt' IDENT params '->' type_ref uses_clause? ':' INDENT prompt_body DEDENT

prompt_body       ::= (role_clause | template_line)+

role_clause       ::= ('system' | 'user' | 'assistant') ':' template_expr

template_line     ::= template_expr NEWLINE

template_expr     ::= STRING_LITERAL ('+' (STRING_LITERAL | IDENT) )*
```

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

annotation        ::= '@' IDENT ('(' annotation_args? ')')?

annotation_args   ::= annotation_arg (',' annotation_arg)*

annotation_arg    ::= IDENT ':' expr | expr

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

```ebnf
server_decl       ::= 'server' IDENT ':' INDENT route_decl+ DEDENT

route_decl        ::= 'route' STRING_LITERAL '->' IDENT NEWLINE   # path → handler agent

schedule_decl     ::= 'schedule' STRING_LITERAL ('zone' STRING_LITERAL)? '->' IDENT '(' arg_list? ')' NEWLINE
```

## Test surface

```ebnf
eval_decl         ::= 'eval' IDENT ':' INDENT eval_body DEDENT
test_decl         ::= 'test' IDENT ':' INDENT block DEDENT
fixture_decl      ::= 'fixture' IDENT ':' INDENT fixture_body DEDENT
mock_decl         ::= 'mock' IDENT ':' INDENT mock_body DEDENT

eval_body         ::= (assertion | stmt)+

# Fixture and mock bodies share their shape with `block` today —
# named separately here so a future fixture-/mock-specific extension
# (custom `@fixture` annotations, mock-call expectation syntax) has a
# place to land without renaming references throughout the doc.
fixture_body      ::= block
mock_body         ::= block

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

for_stmt          ::= 'for' IDENT 'in' expr ':' INDENT block DEDENT

approve_stmt      ::= 'approve' IDENT '(' arg_list? ')' NEWLINE

break_stmt        ::= 'break' NEWLINE
continue_stmt     ::= 'continue' NEWLINE
pass_stmt         ::= 'pass' NEWLINE

assign_stmt       ::= ('let' IDENT (':' type_ref)? | IDENT) '=' expr NEWLINE

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
unary_expr        ::= ('-' | '+')? postfix_expr
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
                    | struct_literal
                    | match_expr
                    | retry_expr

literal           ::= INT | FLOAT | STRING | 'true' | 'false' | 'Nothing'

list_literal      ::= '[' (expr (',' expr)*)? ']'

map_literal       ::= '{' (map_entry (',' map_entry)*)? '}'
map_entry         ::= expr ':' expr

struct_literal    ::= IDENT '{' field_init (',' field_init)* '}'
field_init        ::= IDENT (':' expr)?
                    | '..' expr             # spread / update

match_expr        ::= 'match' expr ':' INDENT match_arm+ DEDENT
match_arm         ::= pattern ('if' expr)? '->' expr NEWLINE

pattern           ::= literal_pattern
                    | IDENT                                  # binding
                    | IDENT '(' pattern (',' pattern)* ')'   # sum-type variant
                    | IDENT '{' field_pattern (',' field_pattern)* (',' '..')? '}'
                    | '_'                                    # wildcard

literal_pattern   ::= literal

field_pattern     ::= IDENT (':' pattern)?

retry_expr        ::= 'try' expr 'on' 'error' 'retry' INT 'times'
                      ('backoff' ('linear' | 'exponential') '(' arg_list ')')?
```

## Lexical tokens

Keywords (reserved): `agent`, `tool`, `prompt`, `eval`, `test`,
`fixture`, `mock`, `server`, `route`, `schedule`, `zone`, `type`,
`session`, `memory`, `import`, `as`, `pub`, `extern`, `extend`,
`public`, `package`, `use`, `try`, `on`, `error`, `retry`, `times`,
`backoff`, `linear`, `exponential`, `approve`, `dangerous`, `effect`,
`uses`, `assert`, `assert_snapshot`, `model`, `requires`,
`progressive`, `below`, `rollout`, `ensemble`, `vote`, `adversarial`,
`propose`, `challenge`, `adjudicate`, `if`, `else`, `for`, `in`,
`return`, `yield`, `break`, `continue`, `pass`, `replay`, `when`,
`true`, `false`, `Nothing`, `and`, `or`, `not`, `let`.

Identifiers: `[A-Za-z_][A-Za-z0-9_]*` excluding keywords.

Numeric literals: `INT` is `[0-9]+` or `0x[0-9a-fA-F]+`; `FLOAT` is
`[0-9]+\.[0-9]+(e[+-]?[0-9]+)?`.

String literals: `"..."` (single-line, escape-processed) or
`"""..."""` (multi-line, raw).

## Authoritative source

The parser at `crates/corvid-syntax/src/parser/` is the source of
truth. This grammar was extracted from it; if a future parser change
diverges, the drift-gate test fails and one of the two must change.
The cross-reference table from production rules to parser-test fns
lives in
[`crates/corvid-syntax/src/parser/tests.rs`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/crates/corvid-syntax/src/parser/tests.rs).
