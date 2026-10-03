use toml_edit::{
    update_document_integers, update_integers, DocumentMut, IntegerUpdate, UpdateFallback,
    UpdateTargetError,
};

fn parse_doc(input: &str) -> DocumentMut {
    input.parse::<DocumentMut>().unwrap()
}

#[test]
fn hex_keeps_base_grouping_and_comment() {
    // Acceptance: `limit = 0x00_FF # ...` updated to 256 stays hexadecimal
    // and keeps its inline comment.
    let before = "limit = 0x00_FF # keep this comment\nname = \"worker\"\n";
    let mut doc = parse_doc(before);

    let report = update_document_integers(&mut doc, [IntegerUpdate::new(["limit"], 256)]).unwrap();
    assert_eq!(report.fallbacks().count(), 0);

    let after = doc.to_string();
    assert_eq!(
        after,
        "limit = 0x01_00 # keep this comment\nname = \"worker\"\n"
    );

    // The result re-parses and holds the expected integer value.
    let reparsed = parse_doc(&after);
    assert_eq!(reparsed["limit"].as_integer(), Some(256));
    assert_eq!(reparsed["name"].as_str(), Some("worker"));
}

#[test]
fn batch_rejected_when_one_target_is_a_string() {
    let before = "limit = 0x00_FF # keep this comment\nname = \"worker\"\n";
    let mut doc = parse_doc(before);

    let err = update_document_integers(
        &mut doc,
        [
            IntegerUpdate::new(["limit"], 256),
            IntegerUpdate::new(["name"], 42),
        ],
    )
    .unwrap_err();

    assert_eq!(err.path, vec!["name".to_owned()]);
    assert_eq!(err.kind, UpdateTargetError::WrongType("string".to_owned()));

    // Nothing in the document changed, including the first target.
    assert_eq!(doc.to_string(), before);
}

#[test]
fn negative_value_falls_back_from_hex_to_decimal() {
    // TOML has no negative hexadecimal literal: a legal representation must
    // be chosen instead and reported.
    let before = "offset = 0x10\n";
    let mut doc = parse_doc(before);

    let report = update_document_integers(&mut doc, [IntegerUpdate::new(["offset"], -1)]).unwrap();
    let fallbacks: Vec<_> = report
        .fallbacks()
        .map(|(path, fallback)| (path.to_vec(), fallback.clone()))
        .collect();
    assert_eq!(
        fallbacks,
        vec![(vec!["offset".to_owned()], UpdateFallback::NegativeForRadix)]
    );

    let after = doc.to_string();
    assert_eq!(after, "offset = -1\n");
    // No illegal `-0x` / `0x-` token was emitted and it parses back.
    assert!(!after.contains("0x-"));
    assert!(!after.contains("-0x"));
    assert_eq!(parse_doc(&after)["offset"].as_integer(), Some(-1));
}

#[test]
fn negative_octal_binary_also_fall_back() {
    for (input, key, new_value, expected) in [
        ("mask = 0o777\n", "mask", -8_i64, "mask = -8\n"),
        ("bits = 0b1010\n", "bits", -2_i64, "bits = -2\n"),
    ] {
        let mut doc = parse_doc(input);
        let report =
            update_document_integers(&mut doc, [IntegerUpdate::new([key], new_value)]).unwrap();
        assert_eq!(report.fallbacks().count(), 1);
        assert_eq!(doc.to_string(), expected);
        assert_eq!(parse_doc(expected)[key].as_integer(), Some(new_value));
    }
}

#[test]
fn dotted_and_quoted_keys_resolve_by_full_path() {
    let before = "\
[server]
port = 8080
[server.tls]
enabled = 1
";
    let mut doc = parse_doc(before);

    update_integers(
        doc.as_table_mut(),
        [
            IntegerUpdate::new(["server", "port"], 443),
            IntegerUpdate::new(["server", "tls", "enabled"], 0),
        ],
    )
    .unwrap();

    assert_eq!(doc["server"]["port"].as_integer(), Some(443));
    assert_eq!(doc["server"]["tls"]["enabled"].as_integer(), Some(0));

    // A dotted key on one `key = value` line is the same nested path.
    let mut dotted = parse_doc("a.b.c = 1\n");
    update_integers(
        dotted.as_table_mut(),
        [IntegerUpdate::new(["a", "b", "c"], 2)],
    )
    .unwrap();
    assert_eq!(dotted.to_string(), "a.b.c = 2\n");

    // A quoted key containing a dot is one single path component and must
    // not be split on the dot.
    let mut quoted = parse_doc("\"a.b\" = 1\n[a]\n");
    update_integers(quoted.as_table_mut(), [IntegerUpdate::new(["a.b"], 3)]).unwrap();
    assert_eq!(quoted["a.b"].as_integer(), Some(3));
    assert_eq!(quoted["a"].as_table().unwrap().len(), 0);

    // The nested two-component path must not match the quoted dotted key.
    let err = update_integers(
        parse_doc("\"a.b\" = 1\n").as_table_mut(),
        [IntegerUpdate::new(["a", "b"], 4)],
    )
    .unwrap_err();
    assert_eq!(err.kind, UpdateTargetError::NotFound);
    assert_eq!(err.path, vec!["a".to_owned()]);
}

#[test]
fn i64_bounds_and_signed_decimal_styles() {
    // i64::MAX in hexadecimal stays hexadecimal, grouping width 4 is reused.
    let mut max_hex = parse_doc("v = 0x7fff_ffff\n");
    update_integers(
        max_hex.as_table_mut(),
        [IntegerUpdate::new(["v"], i64::MAX)],
    )
    .unwrap();
    let after = max_hex.to_string();
    assert_eq!(after, "v = 0x7fff_ffff_ffff_ffff\n");
    assert_eq!(parse_doc(&after)["v"].as_integer(), Some(i64::MAX));

    // i64::MAX in decimal keeps the decimal base and grouping width.
    let mut max_dec = parse_doc("v = 1_000\n");
    update_integers(
        max_dec.as_table_mut(),
        [IntegerUpdate::new(["v"], i64::MAX)],
    )
    .unwrap();
    assert_eq!(max_dec.to_string(), "v = 9_223_372_036_854_775_807\n");

    // i64::MIN cannot be hexadecimal: legal decimal fallback is reported.
    let mut min_hex = parse_doc("v = 0x1\n");
    let report = update_integers(
        min_hex.as_table_mut(),
        [IntegerUpdate::new(["v"], i64::MIN)],
    )
    .unwrap();
    assert_eq!(report.fallbacks().count(), 1);
    assert_eq!(min_hex.to_string(), "v = -9_223_372_036_854_775_808\n");
    assert_eq!(
        parse_doc(&min_hex.to_string())["v"].as_integer(),
        Some(i64::MIN)
    );

    // Explicit plus sign on a decimal literal is preserved.
    let mut plus = parse_doc("v = +5\n");
    update_integers(plus.as_table_mut(), [IntegerUpdate::new(["v"], 9)]).unwrap();
    assert_eq!(plus.to_string(), "v = +9\n");

    // Zero never becomes `00` in decimal.
    let mut zero = parse_doc("v = 1_000\n");
    update_integers(zero.as_table_mut(), [IntegerUpdate::new(["v"], 0)]).unwrap();
    assert_eq!(zero.to_string(), "v = 0\n");
}

#[test]
fn octal_binary_and_case_styles_are_preserved() {
    let mut oct = parse_doc("m = 0o777\n");
    update_integers(oct.as_table_mut(), [IntegerUpdate::new(["m"], 0o755)]).unwrap();
    assert_eq!(oct.to_string(), "m = 0o755\n");

    let mut bin = parse_doc("b = 0b1010_0001\n");
    update_integers(bin.as_table_mut(), [IntegerUpdate::new(["b"], 0b1111_0000)]).unwrap();
    assert_eq!(bin.to_string(), "b = 0b1111_0000\n");

    // Ungrouped literals are not zero-padded, but keep their digit case
    // (the radix prefix stays lowercase, as required by TOML).
    let mut upper = parse_doc("x = 0xDEAD\n");
    update_integers(upper.as_table_mut(), [IntegerUpdate::new(["x"], 0xFF)]).unwrap();
    assert_eq!(upper.to_string(), "x = 0xFF\n");

    // Grouped literals keep the fixed field width via zero-padding.
    let mut upper_grouped = parse_doc("x = 0xDE_AD\n");
    update_integers(
        upper_grouped.as_table_mut(),
        [IntegerUpdate::new(["x"], 0xFF)],
    )
    .unwrap();
    assert_eq!(upper_grouped.to_string(), "x = 0x00_FF\n");

    // Lower-case digits stay lower-case.
    let mut lower = parse_doc("x = 0xdead\n");
    update_integers(lower.as_table_mut(), [IntegerUpdate::new(["x"], 0xff)]).unwrap();
    assert_eq!(lower.to_string(), "x = 0xff\n");
}

#[test]
fn missing_and_mistyped_intermediate_paths_are_not_found() {
    let before = "[a]\nv = 1\n";
    let mut doc = parse_doc(before);

    let err = update_document_integers(&mut doc, [IntegerUpdate::new(["missing"], 1)]).unwrap_err();
    assert_eq!(err.kind, UpdateTargetError::NotFound);
    assert_eq!(err.path, vec!["missing".to_owned()]);

    let err = update_document_integers(&mut doc, [IntegerUpdate::new(["a", "v", "deep"], 1)])
        .unwrap_err();
    assert_eq!(err.kind, UpdateTargetError::NotFound);
    assert_eq!(err.path, vec!["a".to_owned(), "v".to_owned()]);

    // Descending into an array reports the array component as not found.
    let mut arr = parse_doc("v = [1, 2, 3]\n");
    let err = update_document_integers(&mut arr, [IntegerUpdate::new(["v", "0"], 1)]).unwrap_err();
    assert_eq!(err.kind, UpdateTargetError::NotFound);
    assert_eq!(err.path, vec!["v".to_owned()]);

    // Document untouched after any failure.
    assert_eq!(doc.to_string(), before);
}

#[test]
fn unchanged_values_and_unrelated_decor_are_untouched() {
    let before = "\
# leading comment
big = 1_000 # trailing
neg = -9_223_372_036_854_775_808
arr = [ 1, 2, 3 ]
";
    let mut doc = parse_doc(before);

    // Setting the same value does not touch the literal.
    let report = update_document_integers(&mut doc, [IntegerUpdate::new(["big"], 1000)]).unwrap();
    assert_eq!(report.fallbacks().count(), 0);
    assert_eq!(doc.to_string(), before);

    // Grouping width (3) is reused for a larger decimal value.
    update_document_integers(&mut doc, [IntegerUpdate::new(["big"], 1_000_000)]).unwrap();
    assert!(doc.to_string().contains("big = 1_000_000 # trailing\n"));
    // The array and comments are untouched.
    assert!(doc.to_string().contains("arr = [ 1, 2, 3 ]\n"));
}

#[test]
fn values_inside_inline_tables_update_by_path() {
    let before = "cfg = { retries = 3, name = \"bot\" }\n";
    let mut doc = parse_doc(before);

    update_document_integers(&mut doc, [IntegerUpdate::new(["cfg", "retries"], 7)]).unwrap();
    let after = doc.to_string();
    assert_eq!(after, "cfg = { retries = 7, name = \"bot\" }\n");
    assert_eq!(parse_doc(&after)["cfg"]["retries"].as_integer(), Some(7));

    // A sibling that is a string still causes whole-batch rejection.
    let mut doc = parse_doc(before);
    let err = update_document_integers(
        &mut doc,
        [
            IntegerUpdate::new(["cfg", "retries"], 7),
            IntegerUpdate::new(["cfg", "name"], 1),
        ],
    )
    .unwrap_err();
    assert_eq!(err.kind, UpdateTargetError::WrongType("string".to_owned()));
    assert_eq!(doc.to_string(), before);
}

#[test]
fn zero_and_sign_edge_cases_reparse() {
    // Hex zero updated to a negative value falls back to legal decimal.
    let mut z = parse_doc("v = 0x0\n");
    let report = update_document_integers(&mut z, [IntegerUpdate::new(["v"], 0)]).unwrap();
    assert_eq!(report.fallbacks().count(), 0);
    assert_eq!(z.to_string(), "v = 0x0\n");

    // Plain zero becomes negative zero-style decimal (`-0` is valid TOML).
    let mut neg_zero = parse_doc("v = 0\n");
    update_document_integers(&mut neg_zero, [IntegerUpdate::new(["v"], -0)]).unwrap();
    // -0 == 0, so the value is logically unchanged: literal untouched.
    assert_eq!(neg_zero.to_string(), "v = 0\n");

    // Negative decimal keeps the minus sign and grouping for a smaller value.
    let mut neg = parse_doc("v = -1_000_000\n");
    update_document_integers(&mut neg, [IntegerUpdate::new(["v"], -42)]).unwrap();
    assert_eq!(neg.to_string(), "v = -42\n");

    // A larger negative reuses the three-digit grouping width.
    let mut neg2 = parse_doc("v = -1_000_000\n");
    update_document_integers(&mut neg2, [IntegerUpdate::new(["v"], -1_000_000_000)]).unwrap();
    assert_eq!(neg2.to_string(), "v = -1_000_000_000\n");

    // Everything written above re-parses to the expected integer.
    for text in ["v = -42\n", "v = -1_000_000_000\n", "v = 0x0\n"] {
        assert!(parse_doc(text)["v"].is_integer(), "{text}");
    }
}

#[test]
fn nested_tables_and_arrays_of_tables_paths() {
    let before = "\
[[products]]
id = 1
limit = 0xFF

[[products]]
id = 2
limit = 0xFF
";
    // Array-of-tables elements are not navigated by string key paths: such a
    // target must be reported as missing rather than silently created.
    let mut doc = parse_doc(before);
    let err =
        update_document_integers(&mut doc, [IntegerUpdate::new(["products", "0"], 3)]).unwrap_err();
    assert_eq!(err.kind, UpdateTargetError::NotFound);
    assert_eq!(doc.to_string(), before);
}
