// Preferences shim.
//
// Extension preferences run in a separate process with GTK4 and libadwaita,
// which is exactly where the settings app lives. Rather than keeping two
// copies of the settings UI, this page links to the real one — the same
// window the panel menu opens.

import Adw from 'gi://Adw';
import Gio from 'gi://Gio';
import Gtk from 'gi://Gtk';

import {ExtensionPreferences} from 'resource:///org/gnome/Shell/Extensions/js/extensions/prefs.js';

export default class TaskbarPreferences extends ExtensionPreferences {
    fillPreferencesWindow(window) {
        window.set_default_size(560, 480);

        const page = new Adw.PreferencesPage({
            title: 'Taskbar',
            icon_name: 'view-grid-symbolic',
        });

        const settings = new Adw.PreferencesGroup({
            title: 'Tray Settings',
            description: 'Choose which tray icons are shown, in which order, ' +
                'and how large they are. The settings live in the tray daemon, ' +
                'so they apply to every desktop that runs it.',
        });
        settings.add(this._settingsRow(window));
        page.add(settings);

        const about = new Adw.PreferencesGroup({title: 'About'});
        about.add(new Adw.ActionRow({
            title: 'Tray Daemon',
            subtitle: 'Provides the icons on the panel (dev.taskbar.Daemon)',
        }));
        page.add(about);

        window.add(page);
    }

    _settingsRow(window) {
        const row = new Adw.ActionRow({
            title: 'Open Tray Settings',
            subtitle: 'Icons, order, size and panel placement',
            activatable: true,
        });

        const button = new Gtk.Button({
            label: 'Open',
            valign: Gtk.Align.CENTER,
            css_classes: ['suggested-action'],
        });
        button.connect('clicked', () => this._openSettings(window));
        row.add_suffix(button);
        row.connect('activated', () => this._openSettings(window));
        return row;
    }

    _openSettings(window) {
        try {
            const process = Gio.Subprocess.new(
                ['taskbar-settings'], Gio.SubprocessFlags.NONE);
            process.wait_check_async(null, (_source, result) => {
                try {
                    process.wait_check_finish(result);
                } catch (error) {
                    window.add_toast(new Adw.Toast({
                        title: `Could not start taskbar-settings: ${error.message}`,
                    }));
                }
            });
        } catch (error) {
            window.add_toast(new Adw.Toast({
                title: `Could not start taskbar-settings: ${error.message}`,
            }));
        }
    }
}
