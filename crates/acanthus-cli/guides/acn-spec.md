# ACN syntax specification, version 0

ACN is the markup of Acanthus. A `.acn` file describes a tree of components and compiles to the same builder calls as the Rust API. The syntax stays close to QML so that porting is mechanical. There is no scripting: values are expressions of a small typed language with a whitelist of functions.

The version of a file is its first statement, `acn 0`. A file without it is read as the current version. A reader rejects a newer version with a located error.

## Lexical rules

- UTF-8 text. Line comments `// ...` and block comments `/* ... */` (not nested) are whitespace.
- Identifiers: `[A-Za-z_][A-Za-z0-9_]*`. A component name starts with an upper-case letter; properties, ids and functions are `snake_case` and start with a lower-case letter or `_`.
- Numbers: `12`, `0.5`, `.5`, `1e3`. All numbers are `f64` in the language.
- Strings: `"..."` with the escapes `\" \\ \n \t \u{hex}`. A string may not contain a raw newline.
- Colours: `#rgb`, `#rrggbb`, `#rrggbbaa`.
- Keywords: `acn`, `id`, `property`, `Behavior`, `on`, `true`, `false`, `null`.
- Punctuation: `{ } ( ) [ ] : ; , . ? ! + - * / % < > <= >= == != && ||`.
- Members of a component are separated by a newline or `;`. A line that starts with a binary operator, `(`, `[` or `.` continues the previous expression only inside parentheses or brackets.

## Grammar

```
file      = [ "acn" number ] component
component = Name "{" { member } "}"
member    = "id" ":" ident
          | "property" ident ":" expr
          | "on_" ident ":" expr
          | "Behavior" "on" ident "{" component "}"
          | ident ":" expr
          | component
```

- `id: name` gives the component a name that expressions can use as a path root.
- `property name: expr` declares a value that any expression in the file reads by its bare name; names are unique per file.
- `on_event: expr` binds an event, named like the component's event prop (`on_click`, `on_moved`); the expression is evaluated when the event fires and is normally a call (`on_click: action("popup.toggle")`). A handler of an event with an argument (number, bool or index) reads it as `value`.
- `Behavior on prop { Tween { duration: 200 } }` animates changes of `prop` (maps to `Node::animate`).
- `Repeater { count: N  Child { ... } }` expands at compile time into `N` copies of its child components (`N` is a whole number from 0 to 1000 and may use an outer `index`); inside the copies `index` is the copy number from 0. An id declared inside a copy becomes `name_N` (`N` the copy number, `item_2` from outside); inside the copy the plain `name` still works.
- Any other `name: expr` sets a property of the component. The expression re-evaluates when a signal it reads changes.

## Expressions

Precedence from loosest to tightest: `? :`, `||`, `&&`, `== !=`, `< <= > >=`, `+ -`, `* / %`, unary `- !`, postfix call `f(a, b)`, index `a[i]` and path `a.b.c`. `+` joins two texts and adds two numbers; every other mix of types is an error. `&&` and `||` short-circuit.

Values: number, text, bool, colour, list and null. A path resolves against the ids (`id.prop` reads a number, text, bool, colour or index prop), the declared properties, then the globals the host registers (`theme`, services). Calls resolve against the function whitelist: the built-ins `min max clamp abs round floor ceil mix len` plus the functions the host registers (`t`, `fmt_time`, `action`). Every path and function name is checked before the file runs; an unknown one is a located error with the nearest known name.

## Diagnostics

Every error has `file:line:col`, a message and a caret line. A misspelt word reports the nearest known word: `did you mean `color`?`.
