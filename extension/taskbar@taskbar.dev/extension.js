// Taskbar: a system tray for the GNOME panel.
//
// The extension itself is an orchestrator and nothing more. The daemon
// decides what is shown and in which order (it is the one that talks to the
// applications); this file turns its `ListItems` picture into panel
// indicators and forwards input back.

import Gio from 'gi://Gio';
import GObject from 'gi://GObject';
import St from 'gi://St';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';

import {DaemonClient} from './daemon.js';
import {TaskbarIndicator} from './indicator.js';

const ROLE_PREFIX = 'taskbar-';

export default class TaskbarExtension extends Extension {
    enable() {
        this._indicators = new Map();
        this._placeholder = null;
        this._config = null;
        this._busy = false;
        this._queued = false;
        this._client = new DaemonClient((event, detail) => this._onDaemonEvent(event, detail));
    }

    disable() {
        this._client.destroy();
        this._client = null;
        this._config = null;
        this._removeAll();
    }

    // -- daemon events ----------------------------------------------------

    _onDaemonEvent(event, detail) {
        switch (event) {
        case 'items':
            this._refreshItems();
            break;
        case 'config':
            this._refreshConfig();
            break;
        case 'menu':
            this._refreshMenu(detail?.[0]);
            break;
        case 'status':
            if (!this._client.available)
                this._removeAll();
            break;
        }
    }

    async _refreshConfig() {
        try {
            this._config = await this._client.getConfig();
            this._refreshItems();
        } catch (error) {
            console.error(`taskbar: cannot read configuration: ${error.message}`);
        }
    }

    /**
     * Rebuild the icon strip from a complete `ListItems` picture.
     *
     * The daemon sends everything that should be visible, in order, so this
     * is a plain keyed diff — no state is kept about the items themselves.
     */
    async _refreshItems() {
        if (!this._client?.available)
            return;
        if (this._busy) {
            this._queued = true;
            return;
        }
        this._busy = true;
        try {
            const config = this._config ?? await this._client.getConfig();
            this._config = config;
            const scaleFactor = St.ThemeContext.get_for_stage(global.stage).scale_factor;
            const items = await this._client.listItems(config.iconSize * scaleFactor);
            this._applyItems(items, config);
        } catch (error) {
            console.error(`taskbar: cannot list tray items: ${error.message}`);
        } finally {
            this._busy = false;
            if (this._queued) {
                this._queued = false;
                this._refreshItems().catch(logError);
            }
        }
    }

    _applyItems(items, config) {
        const alive = new Set(items.map(item => item.key));
        const box = panelBox(config.panelBox);

        items.forEach((item, index) => {
            let indicator = this._indicators.get(item.key);
            if (!indicator) {
                indicator = new TaskbarIndicator(item, this._client, {iconSize: config.iconSize});
                Main.panel.addToStatusArea(ROLE_PREFIX + item.key, indicator, 0, config.panelBox);
                this._indicators.set(item.key, indicator);
                this._refreshMenu(item.key);
            } else {
                indicator.update(item);
            }
            // Keep the icons in the order the daemon reports, grouped at the
            // configured slot of the panel box.
            box.insert_child_at_index(indicator.container, config.panelPosition + index);
        });

        for (const [key, indicator] of this._indicators) {
            if (!alive.has(key)) {
                indicator.destroy();
                this._indicators.delete(key);
            }
        }

        this._syncPlaceholder(config, items.length);
    }

    async _refreshMenu(key) {
        const indicator = key ? this._indicators.get(key) : null;
        if (!indicator)
            return;
        try {
            const view = await this._client.getMenu(key);
            if (this._indicators.get(key) === indicator)
                indicator.setMenu(view);
        } catch {
            // An item without a menu is perfectly normal.
        }
    }

    // -- empty tray -------------------------------------------------------

    _syncPlaceholder(config, count) {
        const wanted = count === 0 && config.showWhenEmpty;
        if (wanted && !this._placeholder) {
            this._placeholder = new EmptyTrayIndicator();
            Main.panel.addToStatusArea(
                `${ROLE_PREFIX}empty`, this._placeholder, config.panelPosition, config.panelBox);
        } else if (!wanted && this._placeholder) {
            this._placeholder.destroy();
            this._placeholder = null;
        } else if (this._placeholder) {
            panelBox(config.panelBox).insert_child_at_index(
                this._placeholder.container, config.panelPosition);
        }
    }

    _removeAll() {
        for (const indicator of this._indicators.values())
            indicator.destroy();
        this._indicators.clear();
        this._placeholder?.destroy();
        this._placeholder = null;
    }
}

/** Shown in place of the icons when the tray is empty and asked to stay. */
const EmptyTrayIndicator = GObject.registerClass(
class EmptyTrayIndicator extends PanelMenu.Button {
    _init() {
        super._init(0.5, 'Taskbar', false);
        this.add_child(new St.Icon({
            style_class: 'taskbar-placeholder',
            icon_name: 'view-more-symbolic',
            icon_size: 16,
        }));

        const empty = new PopupMenu.PopupMenuItem('No tray items', {reactive: false});
        this.menu.addMenuItem(empty);
        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());
        const settings = new PopupMenu.PopupMenuItem('Tray Settings…');
        settings.connect('activate', () => openSettingsApp());
        this.menu.addMenuItem(settings);
    }
});

function openSettingsApp() {
    try {
        Gio.Subprocess.new(['taskbar-settings'], Gio.SubprocessFlags.NONE);
    } catch (error) {
        console.error(`taskbar: cannot start the settings app: ${error.message}`);
    }
}

function panelBox(name) {
    switch (name) {
    case 'left':
        return Main.panel._leftBox;
    case 'center':
        return Main.panel._centerBox;
    default:
        return Main.panel._rightBox;
    }
}
