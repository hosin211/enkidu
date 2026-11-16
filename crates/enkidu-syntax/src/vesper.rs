//! The Vesper theme's rules for syntax colors, as Shiki has them (`@shikijs/themes` 4.6.0, `dist/vesper.mjs`): which
//! scopes get which color, in the theme's own order. Vesper is by Rauno Freiberg (MIT; see `NOTICE`). Only the colors
//! are here: no italics or bold are drawn from the theme. Generated from the theme file on 12 Nov 2026; not edited by
//! hand.

/// The color of text no rule colors (the theme's `editor.foreground`).
pub const FOREGROUND: &str = "#FFF";

/// Each rule: its scope selectors (one or more, as the theme lists them) and its color.
pub const RULES: &[(&[&str], &str)] = &[
    (&["comment", "punctuation.definition.comment"], "#8b8b8b94"),
    (&["variable", "string constant.other.placeholder", "entity.name.tag"], "#FFF"),
    (&["constant.other.color"], "#FFF"),
    (&["invalid", "invalid.illegal"], "#FF8080"),
    (&["keyword", "storage.type", "storage.modifier"], "#A0A0A0"),
    (
        &[
            "keyword.control",
            "constant.other.color",
            "punctuation.definition.tag",
            "punctuation.separator.inheritance.php",
            "punctuation.definition.tag.html",
            "punctuation.definition.tag.begin.html",
            "punctuation.definition.tag.end.html",
            "punctuation.section.embedded",
            "keyword.other.template",
            "keyword.other.substitution",
        ],
        "#A0A0A0",
    ),
    (&["entity.name.tag", "meta.tag.sgml", "markup.deleted.git_gutter"], "#FFC799"),
    (&["entity.name.function", "variable.function", "support.function", "keyword.other.special-method"], "#FFC799"),
    (&["meta.block variable.other"], "#FFF"),
    (&["support.other.variable", "string.other.link"], "#FFF"),
    (
        &[
            "constant.numeric",
            "support.constant",
            "constant.character",
            "constant.escape",
            "keyword.other.unit",
            "keyword.other",
            "constant.language.boolean",
        ],
        "#FFC799",
    ),
    (
        &[
            "string",
            "constant.other.symbol",
            "constant.other.key",
            "meta.group.braces.curly constant.other.object.key.js string.unquoted.label.js",
        ],
        "#99FFE4",
    ),
    (
        &[
            "entity.name",
            "support.type",
            "support.class",
            "support.other.namespace.use.php",
            "meta.use.php",
            "support.other.namespace.php",
            "markup.changed.git_gutter",
            "support.type.sys-types",
        ],
        "#FFC799",
    ),
    (
        &[
            "source.css support.type.property-name",
            "source.sass support.type.property-name",
            "source.scss support.type.property-name",
            "source.less support.type.property-name",
            "source.stylus support.type.property-name",
            "source.postcss support.type.property-name",
            "source.postcss support.type.property-name",
            "support.type.vendored.property-name.css",
            "source.css.scss entity.name.tag",
            "variable.parameter.keyframe-list.css",
            "meta.property-name.css",
            "variable.parameter.url.scss",
            "meta.property-value.scss",
            "meta.property-value.css",
        ],
        "#FFF",
    ),
    (&["entity.name.module.js", "variable.import.parameter.js", "variable.other.class.js"], "#FF8080"),
    (&["variable.language"], "#A0A0A0"),
    (&["entity.name.method.js"], "#FFFF"),
    (&["meta.class-method.js entity.name.function.js", "variable.function.constructor"], "#FFFF"),
    (
        &[
            "entity.other.attribute-name",
            "meta.property-list.scss",
            "meta.attribute-selector.scss",
            "meta.property-value.css",
            "entity.other.keyframe-offset.css",
            "meta.selector.css",
            "entity.name.tag.reference.scss",
            "entity.name.tag.nesting.css",
            "punctuation.separator.key-value.css",
        ],
        "#A0A0A0",
    ),
    (&["text.html.basic entity.other.attribute-name.html", "text.html.basic entity.other.attribute-name"], "#FFC799"),
    (
        &[
            "entity.other.attribute-name.class",
            "entity.other.attribute-name.id",
            "meta.attribute-selector.scss",
            "variable.parameter.misc.css",
        ],
        "#FFC799",
    ),
    (&["source.sass keyword.control", "meta.attribute-selector.scss"], "#99FFE4"),
    (&["markup.inserted"], "#99FFE4"),
    (&["markup.deleted"], "#FF8080"),
    (&["markup.changed"], "#A0A0A0"),
    (&["string.regexp"], "#A0A0A0"),
    (&["constant.character.escape"], "#A0A0A0"),
    (&["tag.decorator.js entity.name.tag.js", "tag.decorator.js punctuation.definition.tag.js"], "#FFFF"),
    (&["source.js constant.other.object.key.js string.unquoted.label.js"], "#FF8080"),
    (&["source.json meta.structure.dictionary.json support.type.property-name.json"], "#FFC799"),
    (
        &[
            "source.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json support.type.property-name.json",
        ],
        "#FFC799",
    ),
    (
        &[
            "source.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json support.type.property-name.json",
        ],
        "#FFC799",
    ),
    (
        &[
            "source.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json support.type.property-name.json",
        ],
        "#FFC799",
    ),
    (
        &[
            "source.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json support.type.property-name.json",
        ],
        "#FFC799",
    ),
    (
        &[
            "source.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json support.type.property-name.json",
        ],
        "#FFC799",
    ),
    (
        &[
            "source.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json support.type.property-name.json",
        ],
        "#FFC799",
    ),
    (
        &[
            "source.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json support.type.property-name.json",
        ],
        "#FFC799",
    ),
    (
        &[
            "source.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json meta.structure.dictionary.value.json meta.structure.dictionary.json support.type.property-name.json",
        ],
        "#FFC799",
    ),
    (&["text.html.markdown", "punctuation.definition.list_item.markdown"], "#FFF"),
    (&["text.html.markdown markup.inline.raw.markdown"], "#A0A0A0"),
    (&["text.html.markdown markup.inline.raw.markdown punctuation.definition.raw.markdown"], "#FFF"),
    (
        &[
            "markdown.heading",
            "markup.heading | markup.heading entity.name",
            "markup.heading.markdown punctuation.definition.heading.markdown",
            "markup.heading",
            "markup.inserted.git_gutter",
        ],
        "#FFC799",
    ),
    (&["markup.italic"], "#FFF"),
    (&["markup.bold", "markup.bold string"], "#FFF"),
    (
        &[
            "markup.bold markup.italic",
            "markup.italic markup.bold",
            "markup.quote markup.bold",
            "markup.bold markup.italic string",
            "markup.italic markup.bold string",
            "markup.quote markup.bold string",
        ],
        "#FFF",
    ),
    (&["markup.underline"], "#FFC799"),
    (&["markup.quote punctuation.definition.blockquote.markdown"], "#FFF"),
    (&["string.other.link.title.markdown"], "#FFFF"),
    (&["string.other.link.description.title.markdown"], "#A0A0A0"),
    (&["constant.other.reference.link.markdown"], "#FFC799"),
    (&["markup.raw.block"], "#A0A0A0"),
    (&["markup.raw.block.fenced.markdown"], "#00000050"),
    (&["punctuation.definition.fenced.markdown"], "#00000050"),
    (
        &["markup.raw.block.fenced.markdown", "variable.language.fenced.markdown", "punctuation.section.class.end"],
        "#FFF",
    ),
    (&["variable.language.fenced.markdown"], "#FFF"),
    (&["meta.separator"], "#65737E"),
    (&["markup.table"], "#FFF"),
];
