// The panel indicator for one tray item.
//
// Deliberately dumb: it paints an icon the daemon already resolved and
// forwards clicks back to it. It knows nothing about StatusNotifierItem,
// pixmaps or menu protocols — beyond turning a flat list of nodes into popup
// menu entries.

import Clutter from 'gi://Clutter';
import Cogl from 'gi://Cogl';
import Gio from 'gi://Gio';
import GObject from 'gi://GObject';
import St from 'gi://St';

import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';

const FALLBACK_ICON_NAME = 'image-missing';

export const TaskbarIndicator = GObject.registerClass(
class TaskbarIndicator extends PanelMenu.Button {
    /**
     * @param {object} item one item of `ListItems`
     * @param {import('./daemon.js').DaemonClient} daemon
     * @param {object} options `iconSize` in logical pixels
     */
    _init(item, daemon, options) {
        super._init(0.5, item.accessibleName || item.title, false);
        this._daemon = daemon;
        this._item = item;
        this._iconSize = options.iconSize;
        this._menuRevision = -1;

        this._icon = new St.Icon({
            style_class: 'taskbar-icon',
            fallback_icon_name: FALLBACK_ICON_NAME,
            x_align: Clutter.ActorAlign.CENTER,
            y_align: Clutter.ActorAlign.CENTER,
        });
        this.add_child(this._icon);

        // Telling the application when its menu is on screen is part of the
        // menu protocol, so it goes through the daemon like everything else.
        this.menu.connect('open-state-changed', (_menu, open) => {
            this._daemon.menuEvent(this.key, 0, open ? 'opened' : 'closed', 0);
        });

        this.update(item);
    }

    get key() {
        return this._item.key;
    }

    get menuRevision() {
        return this._menuRevision;
    }

    /** Show a new picture of the item; safe to call on every update. */
    update(item) {
        this._item = item;
        this._icon.set({
            width: this._iconSize,
            height: this._iconSize,
            icon_size: this._iconSize,
            content: null,
            gicon: null,
        });

        const {name, themePath, width, height, rowStride, data} = item.icon;
        if (data) {
            // Raw pixels as the application sent them: ARGB32 in network byte
            // order, which is what Cogl calls ARGB_8888. Handing them to
            // St.ImageContent like this is why no conversion happens anywhere.
            const content = new St.ImageContent({
                preferredWidth: width,
                preferredHeight: height,
            });
            // GNOME 48 added the compositor's Cogl context as first argument.
            const args = [];
            const backend = global.stage?.context?.get_backend?.();
            if (content.set_bytes.length === 6 && backend?.get_cogl_context)
                args.push(backend.get_cogl_context());
            args.push(data, Cogl.PixelFormat.ARGB_8888, width, height, rowStride);
            content.set_bytes(...args);
            this._icon.set({
                content,
                contentGravity: Clutter.ContentGravity.RESIZE_ASPECT,
            });
        } else if (name) {
            // No pixels: fall back to the icon theme.
            this._icon.gicon = resolveThemedIcon(name, themePath, this._iconSize);
        }

        this.accessible_name = item.accessibleName || item.title;
    }

    /** Rebuild the popup menu from a `GetMenu` reply. */
    setMenu(view) {
        this._menuRevision = view.revision;
        this.menu.removeAll();
        const items = buildMenuNodes(view.nodes, (nodeId, eventId) => {
            this._daemon.menuEvent(this.key, nodeId, eventId, 0);
            this.menu.close();
        });
        for (const item of items)
            this.menu.addMenuItem(item);
    }

    // -- input ----------------------------------------------------------

    vfunc_event(event) {
        // Touch: open the menu, like the shell does for panel buttons.
        if (event.type() === Clutter.EventType.TOUCH_BEGIN && this.menu.numMenuItems)
            this.menu.toggle();
        return Clutter.EVENT_PROPAGATE;
    }

    vfunc_button_press_event(event) {
        this._pressCoords = event.get_coords();
        // The default behaviour of a panel button (opening its menu) is
        // replaced here: the tray protocol gives every button a meaning.
        return Clutter.EVENT_STOP;
    }

    vfunc_button_release_event(event) {
        const [x, y] = event.get_coords();
        const time = event.get_time();

        switch (event.get_button()) {
        case Clutter.BUTTON_MIDDLE:
            this._daemon.secondaryActivate(this.key, x, y, time);
            break;
        case Clutter.BUTTON_SECONDARY:
            this._showMenu(x, y);
            break;
        case Clutter.BUTTON_PRIMARY:
        default:
            if (this._item.itemIsMenu)
                this._showMenu(x, y);
            else
                this._daemon.activate(this.key, x, y, time);
            break;
        }
        return Clutter.EVENT_STOP;
    }

    vfunc_scroll_event(event) {
        if (event.get_scroll_direction() !== Clutter.ScrollDirection.SMOOTH)
            return Clutter.EVENT_PROPAGATE;
        const [deltaX, deltaY] = event.get_scroll_delta();
        this._daemon.scroll(this.key, deltaX, deltaY);
        return Clutter.EVENT_STOP;
    }

    _showMenu(x, y) {
        if (this.menu.numMenuItems) {
            this.menu.open();
        } else if (this._item.hasMenu) {
            // The menu has not arrived yet; ask the application directly.
            this._daemon.contextMenu(this.key, x, y);
        }
    }
});

/**
 * A `Gio.Icon` for a themed name, honouring the extra search path tray items
 * are allowed to ask for.
 *
 * @param {string} name icon name, or an absolute file path
 * @param {string} themePath extra icon theme directory, may be empty
 * @param {number} size icon size in logical pixels
 * @returns {Gio.Icon}
 */
function resolveThemedIcon(name, themePath, size) {
    if (name.startsWith('/'))
        return new Gio.FileIcon({file: Gio.File.new_for_path(name)});

    if (themePath) {
        const theme = new St.IconTheme();
        theme.set_search_path([themePath]);
        const info = theme.lookup_icon(name, size, 0);
        const filename = info?.get_filename?.();
        if (filename)
            return new Gio.FileIcon({file: Gio.File.new_for_path(filename)});
    }
    return new Gio.ThemedIcon({name});
}

/**
 * Turn the flat node list of a menu into popup menu items.
 *
 * Pure: nodes and an `activate` callback in, widgets out — no daemon calls
 * and no global state.
 *
 * @param {Array<object>} nodes flat, pre-order list of menu nodes
 * @param {(nodeId: number, eventId: string) => void} activate
 * @returns {Array<PopupMenu.PopupBaseMenuItem>}
 */
export function buildMenuNodes(nodes, activate) {
    const byParent = new Map();
    for (const node of nodes) {
        if (!byParent.has(node.parent))
            byParent.set(node.parent, []);
        byParent.get(node.parent).push(node);
    }

    const buildLevel = parentId => (byParent.get(parentId) ?? [])
        .filter(node => node.visible)
        .map(node => buildNode(node, activate, buildLevel));

    return buildLevel(0);
}

function buildNode(node, activate, buildLevel) {
    if (node.kind === 'separator')
        return new PopupMenu.PopupSeparatorMenuItem();

    if (node.hasChildren) {
        const submenu = new PopupMenu.PopupSubMenuMenuItem(node.label);
        for (const child of buildLevel(node.id))
            submenu.menu.addMenuItem(child);
        return submenu;
    }

    const item = new PopupMenu.PopupMenuItem(node.label);
    item.setSensitive(node.enabled);
    if (node.toggleState === 1) {
        item.setOrnament(node.toggleType === 'radio'
            ? PopupMenu.Ornament.DOT
            : PopupMenu.Ornament.CHECK);
    }
    item.connect('activate', () => activate(node.id, 'clicked'));
    return item;
}
