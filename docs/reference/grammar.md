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

# `visibility` applies to type / store / tool / prompt / agent /
# effect / model declarations (and annotation-prefixed agents).
# `public effect x:` / `public model m:` (slice 45o) make the row
# importable via `use`; imported PUBLIC effects join the importing
# file's effect registry, so `uses <imported_effect>` composes
# exactly like a local declaration. Imports, extends, and the test
# surface cannot be `public`; import re-export is not scheduled.
decl              ::= visibility? (
                        type_decl
                      | store_decl
                      | table_decl
                      | database_decl
                      | tool_decl
                      | prompt_decl
                      | agent_decl
                      | component_decl
                      | fn_decl
                      | effect_decl
                      | model_decl
                      | server_decl
                      )
                    | import_decl
                    | browser_callback_decl
                    | browser_lifecycle_decl
                    | identity_decl
                    | connector_decl
                    | schedule_decl
                    | eval_decl
                    | test_decl
                    | fixture_decl
                    | mock_decl
                    | extend_decl

visibility        ::= 'public' ('(' 'package' ')')?
```

## Imports

Local modules, package URIs (`corvid://name@version`), remote URLs
(hash-pinned), and external-ecosystem imports all use string targets;
the `use` list is braceless with optional per-item aliases. Only a Python import can end with an indented signature block. Declare each Python function once, with its parameters and return type. A remote URL requires a SHA-256 pin with exactly 64 hexadecimal digits. Quote a digest that starts with a digit. Package imports use the digest in `Corvid.lock`.

```ebnf
import_decl       ::= 'import' import_target import_constraint* ('as' IDENT)? ('effects' ':' IDENT (',' IDENT)*)? ('use' import_list)? (':' NEWLINE INDENT python_signature+ DEDENT | NEWLINE)

import_target     ::= STRING_LITERAL             # "./module", "corvid://pkg@1.0.0", "https://…"
                    | 'python' STRING_LITERAL    # external ecosystem: import python "mylib"

import_constraint ::= 'requires' annotation+ | 'hash' ':' 'sha256' ':' (STRING_LITERAL | IDENT)

python_signature  ::= 'fn' IDENT params '->' type_ref NEWLINE

import_list       ::= import_item (',' import_item)*

import_item       ::= IDENT ('as' IDENT)?
```

## Types and stores

```ebnf
type_decl         ::= 'type' IDENT ':' INDENT type_field+ DEDENT
                    | 'choice' IDENT ':' INDENT choice_variant+ DEDENT
                    | 'type' IDENT '=' type_alias_body NEWLINE
choice_variant    ::= IDENT ('(' field_list ')')? NEWLINE

# A type declaration is a record (field lines) XOR a sum (variant
# lines) — mixing is a parse error. Unit variants (`| Pending`) are
# bare values; payload variants construct with `Approved("alice")`.
# Variant names are file-scope constructors (duplicates across types
# are duplicate-declaration errors).
type_field        ::= IDENT ':' type_ref NEWLINE
                    | '|' IDENT ('(' field_list ')')? NEWLINE       # sum-type variant

field_list        ::= IDENT ':' type_ref (',' IDENT ':' type_ref)*

# Aliases are TRANSPARENT: `type CustomerId = String` makes
# `CustomerId` the same type as `String` everywhere (no newtype
# semantics). Alias cycles are a compile error; an alias is not a
# constructor.
type_alias_body   ::= type_ref

store_decl        ::= ('session' | 'memory') IDENT ':' INDENT type_field+ DEDENT

# B09: a stored record type and the database that holds it.
# `table`, `database`, `key`, `engine`, `location`, `tables`, `env`,
# `renamed`, `from` and `retired` are contextual: they stay ordinary
# identifiers elsewhere, and a field may be named `retired`.
table_decl        ::= 'table' IDENT ':' INDENT (table_field | table_retired)+ DEDENT
table_field       ::= 'key'? IDENT ':' type_ref ('renamed' 'from' IDENT)? NEWLINE
table_retired     ::= 'retired' IDENT (',' IDENT)* NEWLINE
database_decl     ::= 'database' IDENT ':' INDENT database_setting+ DEDENT
database_setting  ::= 'engine' ':' IDENT NEWLINE
                    | 'location' ':' (STRING | 'env' '(' STRING ')') NEWLINE
                    | 'tables' ':' IDENT (',' IDENT)* NEWLINE
```

## Type references

```ebnf
# `(Int, Int) -> Int` is a function type (slice 45j) — the type of
# lambdas and of function-typed locals. Parameters are checked
# contravariantly, the return type covariantly.
type_ref          ::= function_type
                    | IDENT ('.' IDENT)? type_args?

function_type     ::= '(' (type_ref (',' type_ref)*)? ')' '->' type_ref

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
JSON form) — in the single template and in every role block.

```ebnf
prompt_decl       ::= 'prompt' IDENT params '->' type_ref uses_clause? ':' INDENT prompt_body DEDENT

# A prompt body is EITHER one or more role blocks (46b) OR a single
# template line — mixing them is a parse error (give the final
# message a role instead). Role-block prompts need at least one
# non-`system` message. The single-template form renders as one
# user-role message.
prompt_body       ::= role_clause+ | template_line

role_clause       ::= ('system' | 'user' | 'assistant') ':' STRING_LITERAL NEWLINE

template_line     ::= STRING_LITERAL NEWLINE
```

Prompt bodies also accept structured clauses (`requires:`, `route:`,
`progressive:`, `rollout`, `ensemble`, `adversarial:`, `calibrated`,
`cacheable`, stream settings) before the template — documented in
the prompt chapters and the effect spec; their EBNF lands with a
future grammar-expansion pass rather than being half-specified here.

The `with` modifier lines include the sampling overrides shipped in
slice 46a: `with temperature F` (0..=2) and `with top_p F` (0..=1),
beside the existing `with max_tokens N` — plus `with repair N`
(slice 46h): when a structured response fails typed decode, the
runtime re-asks with the schema-violation feedback appended, up to
N extra attempts, each fully traced (replay reproduces the
sequence) and cost-accounted (failed attempts still cost). Precedence at dispatch is
prompt override > model declaration field > provider default; the
resolved values are recorded in the trace's `llm_call` event.

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

# Sampling fields (slice 46a): `temperature:` (0..=2), `top_p:`
# (0..=1), and `max_tokens:` (positive integer) are range-checked
# at the declaration and become the model's dispatch defaults.
# `context_window: N` (slice 46c) drives deterministic oldest-first
# history truncation at dispatch.
```

## Agent declarations

```ebnf
component_decl    ::= 'component' IDENT ':' INDENT 'state' IDENT (':' 'Int')? '=' INT NEWLINE 'text' '(' STRING ')' NEWLINE component_button+ DEDENT
component_button  ::= 'button' '(' STRING ')' 'on' 'click' ':' INDENT IDENT '+=' INT NEWLINE DEDENT

agent_decl        ::= annotation* extern_abi? 'agent' IDENT params '->' type_ref uses_clause? ':' INDENT block DEDENT

# Pure function (slice 45r) — the fourth callable kind. The body is
# statically EFFECT-FREE: it may call other fns and pure builtins,
# but no tools/prompts/agents/fixtures, no `approve`, no
# `ask`/`choose`, no `replay`, no `yield`. Always callable from
# `@deterministic` bodies. No effect row, annotations, or extern
# ABI — none of those apply to a pure function.
fn_decl           ::= 'fn' IDENT params '->' type_ref ':' INDENT block DEDENT

# Annotation arguments are dimensional constraint values, not general
# expressions: `@budget($0.50)`, `@trust(autonomous)`, `@max_steps(10)`.
# Two named-argument annotations exist (slice 45q): `@retry` and
# `@idempotency`. Their names may collide with reserved keywords
# (`retry`); the parser accepts the keyword token there. The checker
# validates max_attempts >= 1 and that `key` names a String/Int
# parameter of the annotated agent. Both are durable-job policies:
# the agent-side defaults for `corvid jobs` enqueue (flag values win).
annotation        ::= '@' IDENT ('(' annotation_args ')')?
                    | '@retry' '(' 'max_attempts' ':' INT
                      (',' 'backoff' ':' ('linear' | 'exponential') INT)? ')'
                    | '@idempotency' '(' 'key' ':' IDENT ')'

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
a `json Type` or `html` response, and a handler body block. HTML handlers return `String`. The handler reads both JSON and form payloads through `body`, and URL query fields through `query`.
A route can require a signed-in caller, a role, or a permission. Join requirements
with `and`, as in `requires authenticated and permission("status.read")`.
The `zone` clause
on schedules is mandatory.

```ebnf
server_decl       ::= 'server' IDENT ':' INDENT route_decl+ DEDENT

route_decl        ::= (upload_annotation NEWLINE (approval_annotation NEWLINE)?
                      | approval_annotation NEWLINE (upload_annotation NEWLINE)?)?
                      'route' http_method STRING_LITERAL
                      (route_query route_payload? | route_payload route_query?)?
                      '->' ('json' type_ref | 'html') uses_clause? route_policy? ':' INDENT block DEDENT

route_query       ::= 'query' type_ref
route_payload     ::= ('body' | 'form') type_ref
route_policy      ::= 'requires' route_requirement ('and' route_requirement)*
route_requirement ::= 'authenticated' | ('role' | 'permission') '(' STRING_LITERAL ')'

upload_annotation ::= '@' 'upload' '(' (upload_arg (',' upload_arg)* ','?)? ')'
upload_arg        ::= ('max_mb' | 'max_bytes' | 'retention_days') ':' INT | 'mime' ':' STRING_LITERAL
approval_annotation ::= '@' 'approval' '(' approval_arg (',' approval_arg)* ','? ')'
approval_arg      ::= ('role' | 'risk' | 'data') ':' STRING_LITERAL
                    | 'expires_ms' ':' INT | 'max_cost_usd' ':' '$' (FLOAT | INT)
                    | 'irreversible' ':' ('true' | 'false')

http_method       ::= 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE'

schedule_decl     ::= 'schedule' STRING_LITERAL 'zone' STRING_LITERAL
                      '->' IDENT '(' arg_list? ')' uses_clause? NEWLINE
```

## Identity declarations

An identity block declares sign-in providers and can configure sessions,
account linking, provisioning and roles. Within a provisioning block,
`first_login` is required. Both `open` and `invited` require an explicit
tenant assignment. A role's quoted value lists its permissions separated by commas.
The parser recognizes `approval_required` for first login. The checker reports
it as a future feature. Use `open` or `invited` for a runnable application.

```ebnf
identity_decl     ::= 'identity' IDENT ':' INDENT identity_item+ DEDENT
identity_item     ::= 'provider' identity_provider NEWLINE
                    | 'session' ':' INDENT (session_setting NEWLINE)+ DEDENT
                    | 'linking' ':' INDENT (linking_setting NEWLINE)+ DEDENT
                    | 'provisioning' ':' INDENT (provisioning_setting NEWLINE)+ DEDENT
                    | 'roles' ':' INDENT (IDENT ':' STRING_LITERAL NEWLINE)+ DEDENT
identity_provider ::= 'google' | 'github' | 'microsoft' | 'apple' | 'discord' | 'slack'
                    | 'oidc' STRING_LITERAL 'as' IDENT
session_setting   ::= 'lifetime' ':' INT ('s' | 'm' | 'h' | 'd')
                    | 'same_site' ':' ('strict' | 'lax' | 'none')
                    | ('secure' | 'http_only' | 'rotate_on_privilege_change' | 'insecure_opt_out') ':' ('true' | 'false')
linking_setting   ::= 'email_match' ':' ('never' | 'verified_domain')
                    | 'verified_domains' ':' STRING_LITERAL
provisioning_setting ::= 'first_login' ':' ('open' | 'invited' | 'approval_required')
                    | 'tenant' ':' tenant_assignment
                    | 'default_role' ':' IDENT
tenant_assignment ::= 'fixed' '(' STRING_LITERAL ')' | 'from_invitation'
                    | 'from_claim' '(' STRING_LITERAL ')' 'allow' STRING_LITERAL
```

## Connector declarations

A connector groups HTTP operations under a base URL. Declare the modes
callers may use in `modes`. Credentials refer to named secrets. A request
can send one operation parameter as a JSON body or a form.

```ebnf
connector_decl    ::= 'connector' IDENT ':' INDENT connector_item+ DEDENT
connector_item    ::= 'base_url' ':' STRING_LITERAL NEWLINE
                    | 'auth' ':' connector_auth NEWLINE
                    | ('retry' | 'circuit_breaker') ':' INT NEWLINE
                    | 'rate_limit' ':' INT 'per' INT 's'? NEWLINE
                    | 'modes' ':' '[' (connector_mode (',' connector_mode)* ','?)? ']' NEWLINE
                    | operation_decl
connector_mode    ::= 'mock' | 'replay' | 'real'
connector_auth    ::= 'bearer' '(' secret_ref ')'
                    | 'header' '(' STRING_LITERAL ',' secret_ref ')'
                    | 'basic' '(' secret_ref ',' secret_ref ')'
secret_ref        ::= 'secret' '(' STRING_LITERAL ')'
operation_decl    ::= 'operation' IDENT params '->' type_ref 'dangerous'? uses_clause? ':'
                      INDENT http_method STRING_LITERAL (('body' | 'form') IDENT)? NEWLINE
                      operation_item* DEDENT
operation_item    ::= 'on' 'status' INT '->' IDENT NEWLINE | 'mock' ':' expr NEWLINE
                    | 'async' ':' INDENT protocol_item+ DEDENT
protocol_item     ::= ('statuses' | 'terminal') ':' protocol_names NEWLINE
                    | ('initial' | 'deadline_target') ':' IDENT NEWLINE
                    | 'deadline' ':' INT 's'? NEWLINE
                    | 'idempotency' ':' 'intent' 'via' 'header' STRING_LITERAL NEWLINE
                    | ('poll' | 'cancel') http_method STRING_LITERAL NEWLINE
                    | 'every' ':' ('adaptive' | INT 's'?) NEWLINE
                    | 'on_protocol_change' ':' ('refuse' | 'resume') NEWLINE
                    | 'state' IDENT ':' INDENT ('on' IDENT '->' IDENT NEWLINE)+ DEDENT
protocol_names    ::= '[' (IDENT (',' IDENT)* ','?)? ']'
```

An `async` block requires statuses, initial and terminal states, a deadline
and its target, an idempotency header, a polling request and interval, and
an explicit policy for protocol changes. A cancel request is optional.

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

# `assert similar` (deterministic word-set similarity, no LLM
# cost) and `assert judged` (LLM-judge score through the normal
# traced + cost-accounted LLM path) shipped in slice 46h; both
# take a `min` threshold in 0..=1.
assertion         ::= 'assert' expr NEWLINE
                    | 'assert' 'similar' expr ',' expr 'min' FLOAT NEWLINE
                    | 'assert' 'judged' expr ',' STRING_LITERAL 'min' FLOAT NEWLINE
                    | 'assert_snapshot' STRING_LITERAL NEWLINE
```

## Statements

```ebnf
stmt              ::= return_stmt
                    | yield_stmt
                    | if_stmt
                    | for_stmt
                    | while_stmt
                    | destructure_stmt
                    | parallel_stmt
                    | approve_stmt
                    | break_stmt | continue_stmt | pass_stmt
                    | assign_stmt
                    | expr_stmt

return_stmt       ::= 'return' expr? NEWLINE

yield_stmt        ::= 'yield' expr NEWLINE

# `elif` desugars in the parser to `else:` holding a nested `if`,
# so downstream stages see plain if/else trees.
if_stmt           ::= 'if' expr ':' INDENT block DEDENT
                      ('elif' expr ':' INDENT block DEDENT)*
                      ('else' ':' INDENT block DEDENT)?

for_stmt          ::= 'for' IDENT 'in' expr ':' INDENT block DEDENT

# The condition re-evaluates before every iteration and must be
# Bool. `break`/`continue` outside any loop is a COMPILE error.
# `@budget` cost analysis treats a non-zero-cost while body as
# unbounded (no static iteration count).
while_stmt        ::= 'while' expr ':' INDENT block DEDENT

approve_stmt      ::= 'approve' IDENT ('.' IDENT)? '(' arg_list? ')' NEWLINE

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
# Unary `+` is numeric identity — type-checked like `-` (Int/Float
# only), then elided at IR lowering.
unary_expr        ::= ('-' | '+')* postfix_expr
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
                    | lambda_expr
                    | bounded_ask
                    | retry_expr

bounded_ask       ::= 'ask' IDENT '(' expr ')' NEWLINE INDENT 'allow' IDENT 'once' NEWLINE 'within' INT 'seconds' NEWLINE DEDENT

literal           ::= INT | FLOAT | STRING | 'true' | 'false' | 'Nothing'

list_literal      ::= '[' (expr (',' expr)*)? ']'

# Duplicate keys in a literal: the LAST occurrence wins (Python).
# Trailing comma allowed. `{}` is the empty map.
map_literal       ::= '{' (map_entry (',' map_entry)* ','?)? '}'
map_entry         ::= expr ':' expr

# Named-field construction: every declared field must be provided
# (in any order) unless a `..base` spread fills the rest. The
# spread must be LAST and must be the same struct type. A field
# without a value is shorthand for `field: field`. A bare `..`
# (no expression) is only valid when the whole literal is
# reinterpreted as a DESTRUCTURING pattern in statement position.
struct_literal    ::= IDENT '{' field_init (',' field_init)*
                      (',' ('..' expr | '..'))? '}'
field_init        ::= IDENT (':' expr)?

# Statement-position destructuring (irrefutable): shorthand fields
# bind the field name, `field: name` renames, `..` ignores the
# rest. Refutable shapes (literals, nested patterns) belong in
# `match`.
destructure_stmt  ::= struct_literal '=' expr NEWLINE

# `parallel:` (slice 46e) — two or more named arms run
# CONCURRENTLY and join when all complete; arm names bind after
# the block. Each arm's RHS must be a call (wrap richer logic in
# an agent). Effects compose with the parallel operator (cost Sum,
# latency Max, trust Max, confidence Min); arm trace buffers flush
# in ARM ORDER at the join, so replay is deterministic and the
# error rule is arm-ordered. `parallel` is contextual, not
# reserved.
parallel_stmt     ::= 'parallel' ':' INDENT (IDENT '=' expr NEWLINE)+ DEDENT

# `match` is exhaustiveness-checked: sum scrutinees must cover every
# variant irrefutably (or carry a catch-all), Option needs Some+None,
# Result needs Ok+Err, Bool needs true+false, and every other type
# needs a catch-all. Guarded arms never count toward coverage. A bare
# IDENT pattern is a unit-variant test when the name resolves to a
# variant (or None), otherwise it BINDS the scrutinee. `x @ pattern`
# binds and narrows.
match_expr        ::= 'match' expr ':' INDENT match_arm+ DEDENT
match_arm         ::= pattern ('if' expr)? '->' expr NEWLINE

pattern           ::= literal_pattern
                    | '_'                                    # wildcard
                    | IDENT                                  # binding or unit variant
                    | IDENT '@' pattern                      # bind + narrow
                    | IDENT '(' pattern (',' pattern)* ')'   # variant / Some / Ok / Err
                    | IDENT '{' field_pattern (',' field_pattern)* (',' '..')? '}'

literal_pattern   ::= literal | '-' INT | '-' FLOAT

field_pattern     ::= IDENT (':' pattern)?      # bare field name binds it

# A lambda is an EXPRESSION-BODIED anonymous function value.
# Captured outer locals are snapshotted BY VALUE when the lambda
# expression evaluates (heap cells share — the capture copies
# handles, not cells). Unannotated parameter types come from the
# expected function type at the use site (e.g. `map`'s parameter);
# annotations win when both exist. Lambda parameters shadow outer
# locals inside the body.
lambda_expr       ::= 'fn' '(' (lambda_param (',' lambda_param)*)? ')' '->' expr

lambda_param      ::= IDENT (':' type_ref)?

retry_expr        ::= 'try' expr 'on' 'error' 'retry' INT 'times'
                      'backoff' ('linear' | 'exponential') INT    # base delay in ms; backoff is mandatory
```

## Lexical tokens

Keywords (reserved): `agent`, `tool`, `prompt`, `eval`, `test`,
`fixture`, `mock`, `server`, `route`, `schedule`, `zone`, `type`,
`identity`, `connector`, `session`, `memory`, `import`, `as`, `pub`, `extern`, `extend`,
`public`, `package`, `try`, `on`, `error`, `retry`, `times`,
`backoff`, `linear`, `exponential`, `approve`, `dangerous`, `effect`,
`uses`, `assert`, `assert_snapshot`, `model`, `requires`,
`progressive`, `below`, `rollout`, `ensemble`, `vote`, `adversarial`,
`propose`, `challenge`, `adjudicate`, `if`, `else`, `for`, `in`,
`return`, `yield`, `break`, `continue`, `pass`, `while`, `match`,
`fn`, `replay`, `when`, `true`, `false`, `and`, `or`, `not`.

Contextual words (parsed positionally, NOT reserved — they are valid
identifiers elsewhere): `use` (import lists), `Nothing` (the unit
literal/type), `system`/`user`/`assistant` (planned role clauses),
`python` (import source). There is no `let` keyword: bindings are
bare or annotated assignment (decision recorded at ROADMAP slice
45a), so `let` remains an ordinary identifier permanently.

Identifiers: `[A-Za-z_][A-Za-z0-9_]*` excluding keywords.

Decimal digits alone produce an `Int`. A fractional part or an exponent
produces a `Float`, as in `4.5`, `3e-4`, and `1.25E+2`. A fractional
part requires digits after the decimal point. An exponent takes an optional
sign and at least one decimal digit. Float literals must fit in a finite
64-bit floating-point value.

String literals: `"..."` (single-line, escape-processed) or
`"""..."""` (multi-line, raw).

## Authoritative source

The parser at `crates/corvid-syntax/src/parser/` is the source of
truth. This grammar was extracted from it; if a future parser change
diverges, the drift gates fail and one of the two must change. The
per-production parse-evidence snippets live in
[`crates/corvid-syntax/tests/grammar_drift.rs`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/crates/corvid-syntax/tests/grammar_drift.rs).

### Component drag handlers

After the integer state declaration and interpolated text, a component can declare one drag surface followed by one handler.

```text
canvas(width, height)
on drag(parameter: Stroke):
    line(parameter.start, parameter.end)
    state_name += expression
```

For a resizable region use `panel(state_name, minimum, maximum, children)`, where children has type `List<ui.View>`. Sizes and bounds are integer literals from 1 to 8192. Panel bounds must contain the initial width. The optional line statement is available on a canvas. End the handler with an assignment using `=` or `+=`. The expression accepts integer literals, the state name, point coordinates, parentheses, and the arithmetic operators `+`, `-`, and `*`.

The types in `std:ui` are `Point(x: Int, y: Int)` and `Stroke(start: Point, end: Point)`. The compiler renders surfaces with `ui.drag_canvas(label, initial, width, height, handler, draw)` and `ui.drag_panel(label, initial, minimum, maximum, handler, content)`. Both return a `View`. Use `ui.assistant_drag(component, name, description)` to expose the mounted component's action. See [Drawing and resizing with a Stroke](../book/19-browser-views.md#drawing-and-resizing-with-a-stroke).

### Browser callbacks and lifecycle

```ebnf
browser_callback_decl  ::= 'browser' 'callback' IDENT '(' IDENT ':' callback_type ')' '->' callback_type ':' INDENT block DEDENT
callback_type          ::= 'Int' | 'String'
browser_lifecycle_decl ::= 'browser' 'lifecycle' IDENT '(' ')' '->' 'Nothing' uses_clause? ':' INDENT block DEDENT
```

Two top-level declarations in `src/events.cor` create typed component bindings.

The parameter and result of a browser callback must have the same type. A string callback has a pure function body. Import its name into the view and call `name()` to obtain a `ui.TextCallback` for `ui.module_field`. [Typed module fields](../book/19-browser-views.md#read-a-compressed-note-with-a-typed-javascript-package)

```text
browser callback name(parameter: Int) -> Int:
    function_body

browser lifecycle name() -> Nothing:
    lifecycle_body
```

An integer callback has a pure function body and returns the next state. A lifecycle body runs synchronously and may call declared tools. Reaching the end of a lifecycle body completes the hook. In a rendering module, import the declaration and call `name()` to obtain its binding. Use the integer binding with `ui.callback_button` and the lifecycle bindings with `ui.owned_element`. [Typed callbacks and resource ownership](../book/19-browser-views.md#pass-a-browser-callback-and-release-a-resource)

### Browser state bindings

```corvid
public type Dimensions:
    width: Int where between(1, 800)
    height: Int where between(1, 600)

fn size_editor(size: Dimensions) -> ui.View:
    browser state selected = size
    controls = ui.row([ui.local_slider("Width", bind selected.width), ui.local_slider("Height", bind selected.height)])
    return ui.local_selection(selected, controls, ui.record_text("{width} x {height}"))
```

Inside a function, write `browser state name = expression`. The initializer is evaluated once. Its record type supplies the field names and bounds. A field can be a `String` (optionally refined with `len_between`), a `Bool`, a named choice whose variants have no payload, or an `Int` with inclusive ordered bounds in the browser safe integer range (-9007199254740991 through 9007199254740991). A field can also contain another nonempty record of supported fields. Recursive record types are unsupported. Pass the resulting `ui.RecordState` to an editor. A list of these records can initialize a keyed row layout as described below. The compatibility spelling `local type T` and its generated `T_state` helper are still supported. See [browser-local record views](functions.md#browser-local-record-views-in-stdui).

### Browser field bindings

`bind state.field` and nested paths such as `bind settings.profile.name` check each field against the declared record type. The path must end at an editable leaf. The expression produces the dot-separated path as a string for view construction. Direct bindings passed to standard checkbox, slider, swatch and segmented-choice controls are also checked for compatible field types. Ordinary string arguments and text preview templates resolve at mount time. A bind expression selects a field name and does not read its current browser value.

### Keyed rows in view layouts

Inside an indented child list, `for item in records by item.id:` builds one editable row per record in the input list, in source order. The key must be a unique nonempty string field. The loop variable is in scope inside the block, including in `bind item.field` expressions. A row accepts the same editable record fields as a browser record editor. Moving a row preserves its browser-local edits and input focus. Declare `browser state members = workspace.agents` to initialize a typed list of editable records. A typed list expression can also appear directly after `in`. The list is initialized when the page renders.

### Conditions in view layouts

Use `if state.field:` to show child views when a boolean field is true. The compiler checks the field type, including nested paths such as `settings.tools.web_search`.

Inside an indented child list, `if state.field == Variant:` takes one or more indented view expressions. The field can be a nested path rooted in a named browser state. The right-hand name must belong to that field?s payload-free choice type. The browser shows the group while the choice matches and retains its controls and values when hidden. The condition controls visibility only. All fields are still validated on assistant submission. This form supports equality with one named choice variant.

### Indented child blocks

At a line boundary, a function name followed by a colon takes an indented block of one or more child expressions. Corvid evaluates the children in source order and passes them as a single list argument. Each child occupies its own logical line. A child can itself be a block call. The ordinary function signature determines the element and result types.

```corvid
controls = ui.grid:
    ui.field("Agent name", bind settings.name)
    ui.checkbox("Enabled", bind settings.enabled)
```

This is equivalent to `ui.grid([ui.field("Agent name", bind settings.name), ui.checkbox("Enabled", bind settings.enabled)])`. The same syntax works with a developer-defined function, in an assignment, after `return` or `yield`, and as a standalone expression statement. Use an ordinary call for an empty or dynamically assembled list.
