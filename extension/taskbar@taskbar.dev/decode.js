// Decoding of the daemon's replies into plain objects.
//
// Pure and GJS-free on purpose: a reply is anything that offers
// `get_child_value()`, `n_children()`, `unpack()` and `get_data_as_bytes()`,
// which is what `GLib.Variant` provides. That makes the decoding testable
// outside GNOME Shell (see `test/decode.test.mjs`) and immune to the
// `deep_unpack()` pitfalls.
//
// Two of those pitfalls are worth recording:
//
// * `deep_unpack()` turns tuples into arrays, but it does *not* unwrap `v`
//   (variant) values — those stay `GLib.Variant` unless you use
//   `recursive_unpack()`.
// * Anything handed around as a `GLib.Variant` is not iterable, so array
//   destructuring (`function f([a, b])`) throws "is not iterable" on it.
//
// Hence: every field is read with `get_child_value(i)`, one index per field,
// in the order the Rust structs declare them (pinned by the
// `wire_signatures_are_stable` test in taskbar-core).

/** The i-th field of a struct or the i-th element of an array. */
export const child = (value, index) => value.get_child_value(index);

/** The i-th field, unpacked. Only meaningful for scalar fields. */
export const at = (value, index) => child(value, index).unpack();

/** All elements of an `as` array. */
export function strings(value) {
    return Array.from(
        {length: value.n_children()},
        (_ignored, index) => child(value, index).unpack());
}

/** All elements of an `aas` array, e.g. the key chords of a menu entry. */
export function stringLists(value) {
    return Array.from(
        {length: value.n_children()},
        (_ignored, index) => strings(child(value, index)));
}

/** All elements of an `a(...)` array, decoded one by one. */
export function array(value, decode) {
    return Array.from(
        {length: value.n_children()},
        (_ignored, index) => decode(child(value, index)));
}

// -- IconView (ssiiiay) -----------------------------------------------------

export function unpackIcon(value) {
    const data = child(value, 5);
    return {
        name: at(value, 0),
        themePath: at(value, 1),
        width: at(value, 2),
        height: at(value, 3),
        rowStride: at(value, 4),
        // Kept as bytes: St.ImageContent takes its pixels from
        // `get_data_as_bytes()`, and gdk::MemoryTexture likes the same.
        data: data.n_children() > 0 ? data.get_data_as_bytes() : null,
    };
}

// -- ItemView (ssssssbb(ssiiiay)) -------------------------------------------

export function unpackItem(value) {
    return {
        key: at(value, 0),
        appId: at(value, 1),
        title: at(value, 2),
        tooltip: at(value, 3),
        status: at(value, 4),
        accessibleName: at(value, 5),
        hasMenu: at(value, 6),
        itemIsMenu: at(value, 7),
        icon: unpackIcon(child(value, 8)),
    };
}

export function unpackItems(value) {
    return array(value, unpackItem);
}

// -- MenuNodeView (iissbbissaasb) -------------------------------------------

export function unpackNode(value) {
    return {
        id: at(value, 0),
        parent: at(value, 1),
        kind: at(value, 2),
        label: at(value, 3),
        enabled: at(value, 4),
        visible: at(value, 5),
        toggleState: at(value, 6),
        toggleType: at(value, 7),
        iconName: at(value, 8),
        shortcut: stringLists(child(value, 9)),
        hasChildren: at(value, 10),
    };
}

// -- MenuView (sta(iissbbissaasb)) ------------------------------------------

export function unpackMenu(value) {
    return {
        key: at(value, 0),
        revision: Number(at(value, 1)),
        nodes: array(child(value, 2), unpackNode),
    };
}

// -- ConfigView (isasasbsi) -------------------------------------------------

export function unpackConfig(value) {
    return {
        iconSize: at(value, 0),
        sort: at(value, 1),
        order: strings(child(value, 2)),
        hidden: strings(child(value, 3)),
        showWhenEmpty: at(value, 4),
        panelBox: at(value, 5),
        panelPosition: at(value, 6),
    };
}

// -- StatusView (ssbsbitss) -------------------------------------------------

export function unpackStatus(value) {
    return {
        version: at(value, 0),
        watcherName: at(value, 1),
        watcherOwned: at(value, 2),
        watcherOwner: at(value, 3),
        hostRegistered: at(value, 4),
        itemCount: at(value, 5),
        revision: Number(at(value, 6)),
        configPath: at(value, 7),
        lastError: at(value, 8),
    };
}
