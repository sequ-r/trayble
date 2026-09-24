// Client for the taskbar daemon (dev.taskbar.Daemon).
//
// The only place the panel talks to the daemon. Two kinds of pure helpers
// live here as well: they turn the unpacked D-Bus tuples into plain objects,
// so the indicator never has to know that `item[8][4]` is an icon stride.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

Gio._promisify(Gio.DBusConnection.prototype, 'call', 'call_finish');

export const DAEMON_BUS_NAME = 'dev.taskbar.Daemon';
export const DAEMON_OBJECT_PATH = '/dev/taskbar/Daemon';
const DAEMON_INTERFACE = 'dev.taskbar.Daemon';

/**
 * A connection to the daemon that follows its name on the bus.
 *
 * @param {(event: string) => void} onChanged called when something changed;
 *     `event` is one of `'items'`, `'menu'`, `'config'`, `'status'`.
 */
export class DaemonClient {
    constructor(onChanged) {
        this._onChanged = onChanged;
        this._connection = null;
        this._subscriptions = [];
        this._watchId = Gio.bus_watch_name(
            Gio.BusType.SESSION, DAEMON_BUS_NAME,
            Gio.BusNameWatcherFlags.NONE,
            () => this._onNameAppeared(),
            () => this._onNameVanished());
    }

    destroy() {
        Gio.bus_unwatch_name(this._watchId);
        this._watchId = 0;
        this._unwatchSignals();
        this._connection = null;
    }

    get available() {
        return this._connection !== null;
    }

    // -- calls ----------------------------------------------------------

    async call(method, parameters = null) {
        if (!this._connection)
            throw new Error('the tray daemon is not running');
        return this._connection.call(
            DAEMON_BUS_NAME, DAEMON_OBJECT_PATH, DAEMON_INTERFACE,
            method, parameters, null,
            Gio.DBusCallFlags.NONE, -1, null);
    }

    /**
     * Fire and forget: used for input the user will not wait for. Failures
     * are logged and swallowed — a click in a panel must never end in an
     * unhandled rejection.
     */
    _notify(method, parameters = null) {
        return this.call(method, parameters).catch(error =>
            console.debug(`taskbar: ${method} failed: ${error.message}`));
    }

    /** The visible items, with icons resolved at `iconPixelSize` pixels. */
    async listItems(iconPixelSize) {
        const reply = await this.call('ListItems', new GLib.Variant('(i)', [iconPixelSize]));
        const items = reply.get_child_value(0);
        return Array.from(
            {length: items.n_children()},
            (_ignored, index) => unpackItem(items.get_child_value(index)));
    }

    /** The menu of one item, as a flat list of nodes. */
    async getMenu(key) {
        const reply = await this.call('GetMenu', new GLib.Variant('(s)', [key]));
        return unpackMenu(reply.deep_unpack()[0]);
    }

    async getConfig() {
        const reply = await this.call('GetConfig', null);
        return unpackConfig(reply.deep_unpack()[0]);
    }

    async getStatus() {
        const reply = await this.call('GetStatus', null);
        return unpackStatus(reply.deep_unpack()[0]);
    }

    // -- interactions ---------------------------------------------------

    activate(key, x, y, _timestamp) {
        return this._notify('Activate', new GLib.Variant('(sii)', [key, x, y]));
    }

    secondaryActivate(key, x, y, _timestamp) {
        return this._notify('SecondaryActivate', new GLib.Variant('(sii)', [key, x, y]));
    }

    contextMenu(key, x, y) {
        return this._notify('ContextMenu', new GLib.Variant('(sii)', [key, x, y]));
    }

    scroll(key, deltaX, deltaY) {
        const horizontal = Math.abs(deltaX) >= Math.abs(deltaY);
        return this._notify('Scroll', new GLib.Variant('(sis)', [
            key, horizontal ? deltaX : deltaY,
            horizontal ? 'horizontal' : 'vertical',
        ]));
    }

    menuEvent(key, nodeId, eventId, timestamp = 0) {
        return this._notify('MenuEvent', new GLib.Variant('(sisvu)', [
            key, nodeId, eventId,
            new GLib.Variant('u', [0]), timestamp,
        ]));
    }

    // -- wiring ---------------------------------------------------------

    _onNameAppeared() {
        this._connection = Gio.DBus.session;
        this._watchSignals();
        this._onChanged('status');
        this._onChanged('items');
        this._onChanged('config');
    }

    _onNameVanished() {
        this._unwatchSignals();
        this._connection = null;
        this._onChanged('status');
    }

    _watchSignals() {
        const events = {
            ItemsChanged: 'items',
            ConfigChanged: 'config',
            StatusChanged: 'status',
            MenuChanged: 'menu',
        };
        for (const [signal, event] of Object.entries(events)) {
            this._subscriptions.push(this._connection.signal_subscribe(
                DAEMON_BUS_NAME, DAEMON_INTERFACE, signal, DAEMON_OBJECT_PATH,
                null, Gio.DBusSignalFlags.NONE,
                (_connection, _sender, _path, _iface, _name, parameters) =>
                    this._onChanged(event, parameters.deep_unpack())));
        }
    }

    _unwatchSignals() {
        for (const id of this._subscriptions)
            this._connection?.signal_unsubscribe(id);
        this._subscriptions = [];
    }
}

// -- decoding --------------------------------------------------------------
//
// Items are decoded child by child instead of with `deep_unpack()`: the icon
// data must stay a `GLib.Variant`, because `St.ImageContent` takes its pixels
// from `variant.get_data_as_bytes()`. Everything else is plain data.

const child = (value, index) => value.get_child_value(index);
const at = (value, index) => child(value, index).unpack();

function unpackIcon(value) {
    const data = child(value, 5);
    return {
        name: at(value, 0),
        themePath: at(value, 1),
        width: at(value, 2),
        height: at(value, 3),
        rowStride: at(value, 4),
        data: data.n_children() > 0 ? data.get_data_as_bytes() : null,
    };
}

function unpackItem(value) {
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

function unpackNode([id, parent, kind, label, enabled, visible,
    toggleState, toggleType, iconName, shortcut, hasChildren]) {
    return {
        id,
        parent,
        kind,
        label,
        enabled,
        visible,
        toggleState,
        toggleType,
        iconName,
        shortcut,
        hasChildren,
    };
}

function unpackMenu([key, revision, nodes]) {
    return {key, revision: Number(revision), nodes: nodes.map(unpackNode)};
}

function unpackConfig([iconSize, sort, order, hidden, showWhenEmpty, panelBox, panelPosition]) {
    return {
        iconSize,
        sort,
        order,
        hidden,
        showWhenEmpty,
        panelBox,
        panelPosition,
    };
}

function unpackStatus([version, watcherName, watcherOwned, watcherOwner,
    hostRegistered, itemCount, revision, configPath, lastError]) {
    return {
        version,
        watcherName,
        watcherOwned,
        watcherOwner,
        hostRegistered,
        itemCount,
        revision: Number(revision),
        configPath,
        lastError,
    };
}
