//! An IDE's syntax colors: the dark scheme of JetBrains' IDEs, PyCharm among them (item 8.3).
//!
//! The colors are the scheme's own, read on 14 Nov 2026 from the IntelliJ Platform's sources (Apache-2.0; see
//! `NOTICE`): `platform/platform-resources/src/themes/islands/IslandSchemeDark.xml` (the default look since
//! 2025.3), and for Python `python/python-code-insight/resources/colorSchemes/PythonDarcula.xml`. Which token
//! gets which of them is written here for syntect's grammars (Sublime's), whose scopes are not the IDE's own token
//! kinds: the rules follow what the IDE colors, not its code. Only the colors: no italics and no bold, as before.
//!
//! A later rule wins over an earlier one for the same scope, and a longer selector over a shorter one.

/// Plain code (the scheme's `TEXT`).
pub const FOREGROUND: &str = "#BCBEC4";

/// `DEFAULT_KEYWORD`, and what the IDE colors like one.
const KEYWORD: &str = "#CF8E6D";
/// `DEFAULT_STRING`.
const STRING: &str = "#6AAB73";
/// `DEFAULT_NUMBER`.
const NUMBER: &str = "#2AACB8";
/// `DEFAULT_LINE_COMMENT`, `DEFAULT_BLOCK_COMMENT`.
const COMMENT: &str = "#7A7E85";
/// `DEFAULT_DOC_COMMENT`.
const DOC: &str = "#5F826B";
/// `DEFAULT_FUNCTION_DECLARATION`.
const DEFINITION: &str = "#56A8F5";
/// `DEFAULT_METADATA`: decorators and annotations.
const DECORATOR: &str = "#B3AE60";
/// `DEFAULT_INSTANCE_FIELD`, `DEFAULT_CONSTANT`: fields, properties, keys.
const FIELD: &str = "#C77DBB";
/// `HTML_TAG_NAME`, `XML_TAG_NAME`.
const TAG: &str = "#D5B778";
/// `HTML_CUSTOM_TAG_NAME`: a component.
const COMPONENT: &str = "#2FBAA3";
/// An attribute's name (the parent scheme's `HTML_ATTRIBUTE_NAME`).
const ATTRIBUTE: &str = "#BABABA";
/// `PY.SELF_PARAMETER`.
pub const SELF: &str = "#94558D";
/// `PY.BUILTIN_NAME`.
const BUILTIN: &str = "#8888C6";
/// `PY.KEYWORD_ARGUMENT`.
const KEYWORD_ARGUMENT: &str = "#AA4926";
/// `PY.PREDEFINED_DEFINITION`, `PY.PREDEFINED_USAGE`: `__init__` and its kind.
const PREDEFINED: &str = "#B200B2";
/// `JS.REGEXP`.
const REGEXP: &str = "#42C3D4";
/// `PROPERTIES.INVALID_STRING_ESCAPE`: what is wrong.
const WRONG: &str = "#F75464";

/// `ADDED_LINES_COLOR`, `MODIFIED_LINES_COLOR`, `DELETED_LINES_COLOR`: the marks an IDE draws beside the numbers of
/// the lines that differ from the last commit (item 10.2).
pub const ADDED_LINES: &str = "#549159";
pub const MODIFIED_LINES: &str = "#375FAD";
pub const DELETED_LINES: &str = "#868A91";

/// Each rule: its scope selectors and its color.
pub const RULES: &[(&[&str], &str)] = &[
    // ---- every language ----
    (&["comment", "punctuation.definition.comment"], COMMENT),
    (
        &[
            "comment.block.documentation",
            "comment.line.documentation",
            "comment.block.documentation punctuation.definition.comment",
        ],
        DOC,
    ),
    (&["string", "punctuation.definition.string", "storage.type.string"], STRING),
    (&["constant.numeric", "keyword.other.unit"], NUMBER),
    (&["constant.character.escape", "constant.other.placeholder"], KEYWORD),
    (&["keyword", "storage.type", "storage.modifier", "constant.language"], KEYWORD),
    // signs are plain; the words among them are keywords again
    (&["keyword.operator", "keyword.declaration.function.arrow", "storage.type.function.arrow"], FOREGROUND),
    (
        &[
            "keyword.operator.word",
            "keyword.operator.logical.python",
            "keyword.operator.new",
            "keyword.operator.expression",
            "keyword.operator.js",
            "keyword.operator.ts",
            "keyword.operator.tsx",
            "source.sql keyword.operator.logical",
            "keyword.operator.await",
            "keyword.operator.spread.vue",
        ],
        KEYWORD,
    ),
    (&["entity.name.function", "entity.name.method"], DEFINITION),
    (&["variable.annotation", "punctuation.definition.annotation", "meta.annotation", "meta.decorator"], DECORATOR),
    (&["meta.annotation string"], STRING),
    (&["meta.annotation constant.numeric"], NUMBER),
    (&["invalid", "invalid.illegal"], WRONG),
    (
        &[
            "string.regexp",
            "string.regexp punctuation.definition.string",
            "string.regexp keyword",
            "string.regexp keyword.operator",
        ],
        REGEXP,
    ),
    // code inside a string (`f"{x}"`, `${x}`) is code again, and its braces are a keyword's color
    (&["string meta.interpolation", "string source", "meta.interpolation", "meta.template.expression"], FOREGROUND),
    (
        &[
            "punctuation.section.interpolation",
            "punctuation.definition.template-expression",
            "punctuation.section.embedded",
            "punctuation.definition.interpolation",
        ],
        KEYWORD,
    ),
    // ---- Python ----
    (&["source.python variable.language"], SELF),
    (&["source.python support.function", "source.python support.type"], BUILTIN),
    (
        &[
            "source.python support.function.magic",
            "source.python support.variable.magic",
            "source.python entity.name.function.magic",
        ],
        PREDEFINED,
    ),
    (&["source.python meta.function-call.arguments variable.parameter"], KEYWORD_ARGUMENT),
    (&["source.python comment.block.documentation", "source.python string.quoted.docstring"], DOC),
    // ---- JavaScript and TypeScript, where a call's name is colored too ----
    (
        &[
            "source.js variable.function",
            "source.ts variable.function",
            "source.tsx variable.function",
            "source.js support.function",
            "source.ts support.function",
            "source.tsx support.function",
            "source.js entity.name.function",
            "source.ts entity.name.function",
            "source.tsx entity.name.function",
        ],
        DEFINITION,
    ),
    (
        &[
            "source.js meta.property.object",
            "source.ts meta.property.object",
            "source.tsx meta.property.object",
            "variable.other.property",
            "variable.other.object.property",
            "meta.mapping.key.js",
            "meta.object-literal.key",
            "variable.other.readwrite.member",
        ],
        FIELD,
    ),
    (&["variable.language", "support.type.primitive", "support.type.builtin"], KEYWORD),
    // ---- HTML, XML, Vue, JSX ----
    (
        &[
            "entity.name.tag",
            "punctuation.definition.tag",
            "meta.tag.sgml",
            "meta.tag.preprocessor",
            "meta.tag.metadata.doctype",
        ],
        TAG,
    ),
    (&["entity.other.attribute-name", "punctuation.separator.key-value.html"], ATTRIBUTE),
    (&["constant.character.entity"], DEFINITION),
    (&["support.class.component", "entity.name.tag.component", "entity.name.tag.other"], COMPONENT),
    // ---- CSS, SCSS, Less ----
    (
        &[
            "source.css entity.other.attribute-name",
            "source.scss entity.other.attribute-name",
            "source.less entity.other.attribute-name",
            "entity.other.pseudo-class",
            "entity.other.pseudo-element",
            "entity.name.tag.css",
            "punctuation.definition.entity.css",
        ],
        TAG,
    ),
    (&["support.type.property-name", "meta.property-name"], ATTRIBUTE),
    (&["support.constant.property-value", "meta.property-value support.constant"], STRING),
    (
        &[
            "constant.other.color",
            "support.function.css",
            "source.scss support.function",
            "source.css support.function",
        ],
        DEFINITION,
    ),
    (&["keyword.other.important", "keyword.control.at-rule", "punctuation.definition.keyword.css"], KEYWORD),
    (
        &[
            "variable.other.sass",
            "variable.declaration.sass",
            "variable.other.less",
            "variable.scss",
            "variable.other.custom-property",
        ],
        FIELD,
    ),
    // ---- JSON, YAML, TOML, INI ----
    (&["source.json meta.mapping.key string", "source.json meta.mapping.key punctuation.definition.string"], FIELD),
    (
        &[
            "source.yaml entity.name.tag",
            "source.yaml meta.mapping.key string",
            "source.toml entity.name.tag",
            "source.toml meta.mapping.key",
            "source.toml variable.other.key",
            "source.ini entity.name.tag",
            "source.ini variable.other",
        ],
        KEYWORD,
    ),
    (&["entity.name.section", "entity.name.table", "entity.section"], TAG),
    (&["entity.other.anchor", "variable.other.alias", "keyword.control.flow.alias"], FIELD),
    // ---- Markdown ----
    (&["markup.heading", "punctuation.definition.heading", "entity.name.section.markdown"], KEYWORD),
    (&["markup.underline.link", "string.other.link", "meta.link", "constant.other.reference.link"], DEFINITION),
    (&["markup.raw", "markup.raw.inline", "text.html.markdown markup.raw.inline"], FOREGROUND),
    (
        &["punctuation.definition.blockquote", "punctuation.definition.list_item", "markup.list.numbered.bullet"],
        COMMENT,
    ),
    // ---- shell, SQL, Docker ----
    (
        &[
            "source.shell variable.other",
            "source.shell punctuation.definition.variable",
            "variable.parameter.positional",
        ],
        FIELD,
    ),
    (&["source.shell comment.line.shebang", "source.shell meta.shebang"], COMMENT),
    (&["source.sql support.function", "source.sql keyword.other.DML"], KEYWORD),
];
