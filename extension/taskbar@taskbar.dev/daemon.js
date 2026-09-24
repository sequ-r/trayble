// Client for the taskbar daemon (dev.taskbar.Daemon).
//
// The only place the panel talks to the daemon. Two kinds of pure helpers
// support it: `decode.js` turns replies into plain objects, and everything
// here is promise based. The daemon always answers with a complete picture,
// so signals merely say "something changed, ask again".

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

import {
    unpackConfig,
    unpackItems,
    unpackMenu,
    unpackStatus,
} from './decode.js';

Gio._promisify(Gio.DBusConnection.prototype, 'call', 'call_finish');

export const DAEMON_BUS_NAME = 'dev.taskbar.Daemon';
export const DAEMON_OBJECT_PATH = '/dev/taskbar/Daemon';
const DAEMON_INTERFACE = 'dev.taskbar.Daemon';

/**
 * A connection to the daemon that follows its name on the bus.
 *
 * @param {(event: string, detail?: Array) => void} onChanged called when
 *     something changed; `event` is one of `'items'`, `'menu'`, `'config'`,
 *     `'status'`.
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

    /** One reply body: `((...))`, so field 0 is the single return value. */
    async _reply(method, parameters, unpack) {
        const reply = await this.call(method, parameters);
        return unpack(reply.get_child_value(0));
    }

    /** The visible items, with icons resolved at `iconPixelSize` pixels. */
    listItems(iconPixelSize) {
        return this._reply('ListItems',
            new GLib.Variant('(i)', [iconPixelSize]), unpackItems);
    }

    /** The menu of one item, as a flat list of nodes. */
    getMenu(key) {
        return this._reply('GetMenu', new GLib.Variant('(s)', [key]), unpackMenu);
    }

    getConfig() {
        return this._reply('GetConfig', null, unpackConfig);
    }

    getStatus() {
        return this._reply('GetStatus', null, unpackStatus);
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
