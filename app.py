"""
Bluetooth Battery Minimal Circular Widget - Python Overlay
- Pure circular overlay (zero square background, 100% transparent around circle)
- 30% smaller circle
- Clean typography: Percentage inside circle, device name directly under circle
- Draggable anywhere, always-on-top
- Auto-start with Windows (Startup Registry integration)
- Saves and restores position on screen
- Right-click context menu (Refresh, Start with Windows, Always on Top, Close)
"""

import sys
import os
import threading
import time
import json
import winreg
import colorsys
import tkinter as tk
from battery_scanner import scan_devices

# Make high-DPI aware on Windows for crisp fonts and graphics
try:
    import ctypes
    ctypes.windll.shcore.SetProcessDpiAwareness(1)
except Exception:
    try:
        ctypes.windll.user32.SetProcessDPIAware()
    except Exception:
        pass

# Registry constants for Auto-Startup
APP_NAME = "BluetoothBatteryOverlay"
REG_RUN_KEY = r"Software\Microsoft\Windows\CurrentVersion\Run"

# Config path for remembering position
CONFIG_DIR = os.path.join(os.environ.get("APPDATA", os.path.expanduser("~")), "BluetoothBattery")
CONFIG_FILE = os.path.join(CONFIG_DIR, "config.json")


def load_config():
    try:
        if os.path.exists(CONFIG_FILE):
            with open(CONFIG_FILE, "r", encoding="utf-8") as f:
                return json.load(f)
    except Exception:
        pass
    return {}


def save_config(cfg):
    try:
        os.makedirs(CONFIG_DIR, exist_ok=True)
        with open(CONFIG_FILE, "w", encoding="utf-8") as f:
            json.dump(cfg, f, indent=2)
    except Exception:
        pass


def is_startup_enabled():
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, REG_RUN_KEY, 0, winreg.KEY_READ) as key:
            winreg.QueryValueEx(key, APP_NAME)
            return True
    except FileNotFoundError:
        return False
    except Exception:
        return False


def set_startup(enable: bool):
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, REG_RUN_KEY, 0, winreg.KEY_SET_VALUE) as key:
            if enable:
                if getattr(sys, "frozen", False):
                    # Compiled .exe
                    cmd = f'"{sys.executable}"'
                else:
                    # Python script launcher via pythonw
                    pythonw = os.path.join(os.path.dirname(sys.executable), "pythonw.exe")
                    if not os.path.exists(pythonw):
                        pythonw = sys.executable
                    script_path = os.path.abspath(__file__)
                    cmd = f'"{pythonw}" "{script_path}"'
                winreg.SetValueEx(key, APP_NAME, 0, winreg.REG_SZ, cmd)
            else:
                try:
                    winreg.DeleteValue(key, APP_NAME)
                except FileNotFoundError:
                    pass
        return True
    except Exception as e:
        print("Startup registry error:", e)
        return False


def get_battery_color(percent):
    """
    Computes an exact HEX color on a smooth spectrum:
    100% = Emerald Green
     50% = Warm Amber / Yellow
     20% = Coral Orange
      0% = Crimson Red
    """
    pct = max(0.0, min(100.0, float(percent)))
    hue = (pct / 100.0) * 0.37
    r, g, b = colorsys.hsv_to_rgb(hue, 0.88, 0.98)
    return f"#{int(r*255):02x}{int(g*255):02x}{int(b*255):02x}"


class MinimalCircularBatteryOverlay:
    def __init__(self, root):
        self.root = root
        self.root.title("Bluetooth Battery")
        
        # Dimensions (compact circular widget)
        self.width = 130
        self.height = 145
        
        # Windows transparent chroma color
        self.chroma_color = "#010101"
        self.disc_bg = "#12151E"      # Sleek dark disc inside circle for high contrast
        self.track_color = "#232938"  # Subtle dark track ring
        self.text_primary = "#FFFFFF"
        self.text_secondary = "#9AA8BE"
        
        # Configure frameless & transparent window
        self.root.overrideredirect(True)
        self.root.attributes("-topmost", True)
        self.root.attributes("-transparentcolor", self.chroma_color)
        self.root.configure(bg=self.chroma_color)
        
        # Set app icon if available
        icon_path = os.path.join(os.path.dirname(__file__), "app_icon.ico")
        if os.path.exists(icon_path):
            try:
                self.root.iconbitmap(icon_path)
            except Exception:
                pass

        # Load saved position or default to top-right
        self.config = load_config()
        screen_w = self.root.winfo_screenwidth()
        x = self.config.get("pos_x", screen_w - self.width - 30)
        y = self.config.get("pos_y", 50)
        self.root.geometry(f"{self.width}x{self.height}+{x}+{y}")
        
        # State
        self.devices = []
        self.selected_idx = 0
        self.is_scanning = False
        self.always_on_top = True
        
        # Dragging variables
        self.drag_x = 0
        self.drag_y = 0
        
        # Setup UI
        self.setup_ui()
        
        # Initial scan & auto-refresh
        self.trigger_scan()
        self.schedule_auto_refresh()

    def setup_ui(self):
        self.canvas = tk.Canvas(
            self.root,
            width=self.width,
            height=self.height,
            bg=self.chroma_color,
            highlightthickness=0
        )
        self.canvas.pack(fill="both", expand=True)
        
        # Bind dragging anywhere on the widget
        self.canvas.bind("<Button-1>", self.start_drag)
        self.canvas.bind("<B1-Motion>", self.do_drag)
        self.canvas.bind("<ButtonRelease-1>", self.end_drag)
        self.canvas.bind("<Button-3>", self.show_context_menu)
        self.canvas.bind("<Double-Button-1>", lambda e: self.trigger_scan())
        
        # Initial placeholder render
        self.render(70, "Scanning...")

    def build_context_menu(self):
        menu = tk.Menu(self.root, tearoff=0, bg="#181C26", fg="#FFFFFF", activebackground="#3880FF", font=("Segoe UI", 9))
        menu.add_command(label="↻ Refresh Now", command=self.trigger_scan)
        
        # Startup toggle item with checkmark indicator
        startup_state = is_startup_enabled()
        startup_label = "✔ Start with Windows" if startup_state else "Start with Windows"
        menu.add_command(label=startup_label, command=self.toggle_startup_state)
        
        top_label = "✔ Always on Top" if self.always_on_top else "Always on Top"
        menu.add_command(label=top_label, command=self.toggle_topmost)
        
        menu.add_separator()
        menu.add_command(label="✕ Close Widget", command=self.root.destroy)
        return menu

    def toggle_startup_state(self):
        curr = is_startup_enabled()
        set_startup(not curr)

    def start_drag(self, event):
        self.drag_x = event.x
        self.drag_y = event.y

    def do_drag(self, event):
        dx = event.x - self.drag_x
        dy = event.y - self.drag_y
        new_x = self.root.winfo_x() + dx
        new_y = self.root.winfo_y() + dy
        self.root.geometry(f"+{new_x}+{new_y}")

    def end_drag(self, event):
        # Save position to config
        self.config["pos_x"] = self.root.winfo_x()
        self.config["pos_y"] = self.root.winfo_y()
        save_config(self.config)

    def render(self, battery_level, device_name):
        self.canvas.delete("all")
        
        cx = self.width / 2
        cy = 54
        radius = 44       # 30% smaller than previous 65px
        thickness = 7
        
        x1 = cx - radius
        y1 = cy - radius
        x2 = cx + radius
        y2 = cy + radius
        
        # 1. Dark circular disc backdrop
        self.canvas.create_oval(
            x1 - 1, y1 - 1, x2 + 1, y2 + 1,
            fill=self.disc_bg,
            outline="#1F2533",
            width=1
        )
        
        # 2. Gray track ring
        self.canvas.create_arc(
            x1, y1, x2, y2,
            start=0, extent=359.9,
            style="arc",
            width=thickness,
            outline=self.track_color
        )
        
        # 3. Dynamic colored arc
        if battery_level >= 0:
            color = get_battery_color(battery_level)
            extent = -(battery_level / 100.0) * 359.9
            if extent != 0:
                self.canvas.create_arc(
                    x1, y1, x2, y2,
                    start=90,
                    extent=extent,
                    style="arc",
                    width=thickness,
                    outline=color
                )
            
            # Percentage number inside circle center
            self.canvas.create_text(
                cx, cy - 2,
                text=f"{battery_level}%",
                font=("Segoe UI", 16, "bold"),
                fill=self.text_primary,
                anchor="center"
            )
            
            # Subtle small status text inside circle
            self.canvas.create_text(
                cx, cy + 18,
                text="BATTERY",
                font=("Segoe UI", 6, "bold"),
                fill=color,
                anchor="center"
            )
        else:
            color = "#606D80"
            self.canvas.create_text(
                cx, cy,
                text="--%",
                font=("Segoe UI", 16, "bold"),
                fill=self.text_secondary,
                anchor="center"
            )
        
        # 4. Device Name directly underneath the circle
        name_y = cy + radius + 18
        
        # Truncate device name if needed
        clean_name = device_name
        if len(clean_name) > 14:
            clean_name = clean_name[:12] + ".."
            
        # Small pill background for device name
        name_w = 96
        self.draw_pill(
            cx - name_w / 2, name_y - 10,
            cx + name_w / 2, name_y + 11,
            r=8,
            fill="#12151E",
            outline="#1F2533"
        )
        
        # Colored dot next to name
        dot_color = color if battery_level >= 0 else "#606D80"
        self.canvas.create_oval(
            cx - name_w / 2 + 8, name_y - 3,
            cx - name_w / 2 + 14, name_y + 3,
            fill=dot_color,
            outline=""
        )
        
        # Device text
        self.canvas.create_text(
            cx - name_w / 2 + 20, name_y,
            text=clean_name,
            font=("Segoe UI", 8, "bold"),
            fill=self.text_primary,
            anchor="w"
        )

        # Multi-device indicator if > 1 device
        if len(self.devices) > 1:
            self.canvas.create_text(
                cx + name_w / 2 - 10, name_y,
                text="▶",
                font=("Segoe UI", 7, "bold"),
                fill=self.text_secondary,
                anchor="center",
                tags="btn_next"
            )
            self.canvas.tag_bind("btn_next", "<Button-1>", lambda e: self.next_device())

    def draw_pill(self, x1, y1, x2, y2, r=6, **kwargs):
        points = [
            x1 + r, y1,
            x2 - r, y1,
            x2, y1,
            x2, y1 + r,
            x2, y2 - r,
            x2, y2,
            x2 - r, y2,
            x1 + r, y2,
            x1, y2,
            x1, y2 - r,
            x1, y1 + r,
            x1, y1
        ]
        return self.canvas.create_polygon(points, smooth=True, **kwargs)

    def next_device(self):
        if self.devices:
            self.selected_idx = (self.selected_idx + 1) % len(self.devices)
            curr = self.devices[self.selected_idx]
            self.render(curr["battery"], curr["name"])

    def trigger_scan(self):
        if self.is_scanning:
            return
        self.is_scanning = True
        
        def worker():
            devs = scan_devices()
            self.root.after(0, self.on_scan_finished, devs)
            
        threading.Thread(target=worker, daemon=True).start()

    def on_scan_finished(self, devs):
        self.is_scanning = False
        self.devices = devs
        
        if self.devices:
            if self.selected_idx >= len(self.devices):
                self.selected_idx = 0
            curr = self.devices[self.selected_idx]
            self.render(curr["battery"], curr["name"])
        else:
            self.render(-1, "No Device")

    def schedule_auto_refresh(self):
        self.trigger_scan()
        self.root.after(30000, self.schedule_auto_refresh)

    def toggle_topmost(self):
        self.always_on_top = not self.always_on_top
        self.root.attributes("-topmost", self.always_on_top)

    def show_context_menu(self, event):
        menu = self.build_context_menu()
        menu.tk_popup(event.x_root, event.y_root)


def main():
    root = tk.Tk()
    app = MinimalCircularBatteryOverlay(root)
    root.mainloop()


if __name__ == "__main__":
    main()
