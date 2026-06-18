/**
 * The per-grammar value-extraction map (§17.4). The `value` selector identifies a
 * literal by AST node kind, per grammar. Extraction: pre-order DFS over *named*
 * children, take the first matching literal and stop; `array`/collection literals
 * have all whitespace stripped; booleans and null/none are scalars (literal text
 * is the value); if nothing matches, the `value` selector is omitted.
 */

export interface ValueKinds {
  scalar: ReadonlySet<string>;
  string: ReadonlySet<string>;
  collection: ReadonlySet<string>;
}

const s = (...xs: string[]) => new Set(xs);

export const VALUE_MAP: Record<string, ValueKinds> = {
  typescript: {
    scalar: s("number", "true", "false", "null", "undefined"),
    string: s("string"),
    collection: s("array"),
  },
  tsx: {
    scalar: s("number", "true", "false", "null", "undefined"),
    string: s("string"),
    collection: s("array"),
  },
  python: {
    scalar: s("integer", "float", "true", "false", "none"),
    string: s("string"),
    collection: s("list", "tuple", "set", "dictionary"),
  },
  rust: {
    scalar: s("integer_literal", "float_literal", "boolean_literal"),
    string: s("string_literal", "char_literal", "raw_string_literal"),
    collection: s("array_expression"),
  },
  go: {
    scalar: s(
      "int_literal",
      "float_literal",
      "imaginary_literal",
      "true",
      "false",
      "nil",
    ),
    string: s(
      "interpreted_string_literal",
      "raw_string_literal",
      "rune_literal",
    ),
    collection: s("composite_literal"),
  },
  java: {
    scalar: s(
      "decimal_integer_literal",
      "hex_integer_literal",
      "decimal_floating_point_literal",
      "true",
      "false",
      "null_literal",
      "character_literal",
    ),
    string: s("string_literal"),
    collection: s("array_initializer"),
  },
};

/** Whether a node kind carries a literal value for `language`, and its class. */
export function valueClass(
  language: string,
  kind: string,
): "scalar" | "string" | "collection" | null {
  const map = VALUE_MAP[language];
  if (!map) return null;
  if (map.scalar.has(kind)) return "scalar";
  if (map.string.has(kind)) return "string";
  if (map.collection.has(kind)) return "collection";
  return null;
}
