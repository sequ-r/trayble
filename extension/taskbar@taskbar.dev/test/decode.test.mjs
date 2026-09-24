// Tests for the reply decoding.
//
// The stand-in below is deliberately *not* iterable: `GLib.Variant` is not
// iterable either, so any code that array-destructures a variant fails here
// exactly like it fails in GNOME Shell ("is not iterable").
//
//     node --test extension/taskbar@taskbar.dev/test/

import assert from 'node:assert/strict';
import {describe, it} from 'node:test';

import {
    unpackConfig,
    unpackItem,
    unpackItems,
    unpackMenu,
    unpackStatus,
} from '../decode.js';

/** A `GLib.Variant` stand-in: scalar-ish, not iterable. */
class FakeVariant {
    constructor(type, value) {
        this._type = type;
        this._value = value;
    }

    get_child_value(index) {
        assert.ok(Array.isArray(this._value),
            `get_child_value() needs a container, got '${this._type}'`);
        return this._value[index];
    }

    n_children() {
        return this._value.length;
    }

    unpack() {
        assert.ok(!Array.isArray(this._value),
            `unpack() needs a scalar, got '${this._type}'`);
        return this._value;
    }

    get_data_as_bytes() {
        assert.equal(this._type, 'ay');
        return {toArray: () => this._value};
    }
}

const v = (type, value) => new FakeVariant(type, value);

// -- fixtures, shaped exactly like the Rust structs in taskbar-core -------

const PNG_BYTES = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3];

function iconFixture() {
    return v('(ssiiay)', [
        v('s', 'demo-icon'),
        v('s', '/opt/demo/icons'),
        v('i', 2),
        v('i', 2),
        v('ay', PNG_BYTES),
    ]);
}

function itemFixture() {
    return v('(ssssssbb(ssiiay))', [
        v('s', 'demo'),
        v('s', 'demo'),
        v('s', 'Demo'),
        v('s', 'A demo item'),
        v('s', 'Active'),
        v('s', 'Demo is running'),
        v('b', true),
        v('b', false),
        iconFixture(),
    ]);
}

function nodeFixture(id, parent) {
    return v('(iissbbissaasb)', [
        v('i', id),
        v('i', parent),
        v('s', 'standard'),
        v('s', 'Quit'),
        v('b', true),
        v('b', true),
        v('i', 1),
        v('s', 'checkmark'),
        v('s', ''),
        v('aas', [v('as', [v('s', 'Control'), v('s', 'Q')])]),
        v('b', false),
    ]);
}

function menuFixture() {
    return v('(sta(iissbbissaasb))', [
        v('s', 'demo'),
        v('t', 7),
        v('a(iissbbissaasb)', [nodeFixture(1, 0), nodeFixture(2, 0)]),
    ]);
}

function configFixture() {
    return v('(isasasbsi)', [
        v('i', 24),
        v('s', 'manual'),
        v('as', [v('s', 'b'), v('s', 'a')]),
        v('as', [v('s', 'hidden')]),
        v('b', true),
        v('s', 'left'),
        v('i', 3),
    ]);
}

function statusFixture() {
    return v('(ssbsbitss)', [
        v('s', '0.1.0'),
        v('s', 'org.kde.StatusNotifierWatcher'),
        v('b', true),
        v('s', ':1.0'),
        v('b', true),
        v('i', 2),
        v('t', 41),
        v('s', '/home/demo/.config/taskbar/config.toml'),
        v('s', ''),
    ]);
}

// -- tests --------------------------------------------------------------

describe('reply decoding', () => {
    it('decodes an item and keeps its icon bytes', () => {
        const item = unpackItem(itemFixture());

        assert.equal(item.key, 'demo');
        assert.equal(item.hasMenu, true);
        assert.equal(item.itemIsMenu, false);
        assert.equal(item.icon.name, 'demo-icon');
        assert.equal(item.icon.width, 2);
        assert.equal(item.icon.height, 2);
        assert.deepEqual(Array.from(item.icon.data.toArray()), PNG_BYTES,
            'the icon arrives as a PNG image');
    });

    it('leaves the icon data empty when there are no pixels', () => {
        const bare = iconFixture();
        bare._value[4] = v('ay', []);

        const item = unpackItem(v('(ssssssbb(ssiiay))', [
            ...itemFixture()._value.slice(0, 8),
            bare,
        ]));
        assert.equal(item.icon.data, null);
    });

    it('decodes a list of items', () => {
        const items = unpackItems(v('a(ssssssbb(ssiiay))', [
            itemFixture(),
            itemFixture(),
        ]));
        assert.equal(items.length, 2);
        assert.equal(items[1].key, 'demo');
    });

    it('decodes a menu with its key chords', () => {
        const menu = unpackMenu(menuFixture());

        assert.equal(menu.key, 'demo');
        assert.equal(menu.revision, 7);
        assert.equal(menu.nodes.length, 2);
        assert.equal(menu.nodes[0].id, 1);
        assert.equal(menu.nodes[0].parent, 0);
        assert.equal(menu.nodes[0].toggleState, 1);
        assert.deepEqual(menu.nodes[0].shortcut, [['Control', 'Q']]);
    });

    it('decodes the configuration', () => {
        const config = unpackConfig(configFixture());

        assert.equal(config.iconSize, 24);
        assert.equal(config.sort, 'manual');
        assert.deepEqual(config.order, ['b', 'a']);
        assert.deepEqual(config.hidden, ['hidden']);
        assert.equal(config.showWhenEmpty, true);
        assert.equal(config.panelBox, 'left');
        assert.equal(config.panelPosition, 3);
    });

    it('decodes the status', () => {
        const status = unpackStatus(statusFixture());

        assert.equal(status.version, '0.1.0');
        assert.equal(status.watcherOwned, true);
        assert.equal(status.itemCount, 2);
        assert.equal(status.revision, 41);
    });

    it('fails loudly instead of silently on the wrong shape', () => {
        assert.throws(() => unpackItem(itemFixture().get_child_value(0)),
            /needs a container/);
    });
});
