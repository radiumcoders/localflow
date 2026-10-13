"""The app's setup window: where to clean up what you say (on this computer, or
with your API key), then install.py's steps with progress, then how to dictate."""

import os
import platform
import queue
import sys
import threading
import traceback
import tkinter as tk
from pathlib import Path
from tkinter import font as tkfont, ttk

import install

SYSTEM = platform.system()
PROVIDERS = {"Anthropic (Claude Haiku)": "anthropic", "OpenAI": "openai", "OpenRouter": "openrouter",
             "Groq": "groq", "Google Gemini": "gemini", "Mistral": "mistral", "DeepSeek": "deepseek"}
KEYS = {"Windows": ("Ctrl+Shift+Space", "Ctrl+Alt+Space"), "Darwin": ("Option+Shift+Space", "Option+Shift+L")}


class Lines:
    """print() from the install steps, a line at a time into the window."""

    def __init__(self, q):
        self.q, self.buf = q, ""

    def write(self, s):
        self.buf += s
        *lines, self.buf = self.buf.split("\n")
        for line in lines:
            self.q.put(("line", line))
        return len(s)

    def flush(self):
        pass


def run_install(q, api, model, key):
    install.CAPTURE = True
    install.PROGRESS = lambda done, total: q.put(("progress", done, total))
    out = sys.stdout, sys.stderr
    sys.stdout = sys.stderr = Lines(q)
    if key:
        os.environ["LOCALFLOW_API_KEY"] = key  # setup_api saves it
    try:
        hyprland = SYSTEM == "Linux" and bool(os.environ.get("HYPRLAND_INSTANCE_SIGNATURE"))
        rocm = SYSTEM == "Linux" and not api and install.has_amd_gpu()
        q.put(("done", install.install(api, model, rocm=rocm, hyprland=hyprland)))
    except SystemExit as e:
        msg = str(e.code) if e.code not in (None, 0) else "Setup stopped."
        q.put(("error", msg.replace("re-run install.py", "try again").replace("then re-run", "then try again")))
    except Exception as e:
        traceback.print_exc()
        q.put(("error", f"{type(e).__name__}: {e}"))
    finally:
        os.environ.pop("LOCALFLOW_API_KEY", None)
        sys.stdout.flush()
        sys.stdout, sys.stderr = out


class Setup:
    def __init__(self, root):
        self.root, self.q = root, queue.Queue()
        root.title("localflow setup")
        root.minsize(560, 460)
        root.columnconfigure(0, weight=1)
        root.rowconfigure(0, weight=1)
        style = ttk.Style()
        self.font = tkfont.nametofont("TkDefaultFont")
        size = self.font.actual("size")
        style.configure("Title.TLabel", font=(self.font.actual("family"), size + 7, "bold"))
        style.configure("Choice.TRadiobutton", font=(self.font.actual("family"), size + 1, "bold"))
        style.configure("Hint.TLabel", foreground="#666")
        style.configure("Error.TLabel", foreground="#b3261e")
        self.pages = {}
        self.choose_page()
        self.progress_page()
        self.done_page()
        self.show("choose")
        root.after(50, self.poll)

    def show(self, name):
        for page in self.pages.values():
            page.grid_remove()
        self.pages[name].grid(row=0, column=0, sticky="nsew")

    def page(self, name, title, text=None):
        f = ttk.Frame(self.root, padding=24)
        f.columnconfigure(0, weight=1)
        ttk.Label(f, text=title, style="Title.TLabel").grid(row=0, column=0, sticky="w")
        if text:
            ttk.Label(f, text=text, wraplength=500, justify="left").grid(row=1, column=0, sticky="w", pady=(6, 14))
        self.pages[name] = f
        return f

    # --- 1: where to clean up ------------------------------------------------------

    def choose_page(self):
        f = self.page("choose", "Set up localflow",
                      "Talk anywhere you can type, and localflow pastes what you meant: punctuation "
                      "fixed, filler words and false starts gone. Choose where that cleanup runs.")
        self.mode = tk.StringVar(value="local")
        ttk.Radiobutton(f, text="On this computer", value="local", variable=self.mode, style="Choice.TRadiobutton",
                        command=self.mode_changed).grid(row=2, column=0, sticky="w")
        ttk.Label(f, text="Private and free. Downloads about 3.5 GB; works best with 16 GB of memory "
                          "or a graphics card.", style="Hint.TLabel", wraplength=470, justify="left"
                  ).grid(row=3, column=0, sticky="w", padx=(26, 0), pady=(0, 12))
        ttk.Radiobutton(f, text="With your own API key", value="api", variable=self.mode, style="Choice.TRadiobutton",
                        command=self.mode_changed).grid(row=4, column=0, sticky="w")
        ttk.Label(f, text="Fast on any computer. What you say is sent to the provider you choose.",
                  style="Hint.TLabel", wraplength=470, justify="left"
                  ).grid(row=5, column=0, sticky="w", padx=(26, 0), pady=(0, 8))

        api = ttk.Frame(f, padding=(26, 0, 0, 0))
        api.grid(row=6, column=0, sticky="ew")
        api.columnconfigure(1, weight=1)
        self.provider = tk.StringVar(value=next(iter(PROVIDERS)))
        self.model = tk.StringVar()
        self.key = tk.StringVar()
        ttk.Label(api, text="Provider").grid(row=0, column=0, sticky="w", pady=3)
        self.provider_box = ttk.Combobox(api, textvariable=self.provider, values=list(PROVIDERS), state="readonly")
        self.provider_box.grid(row=0, column=1, sticky="ew", padx=(10, 0), pady=3)
        self.provider_box.bind("<<ComboboxSelected>>", lambda e: self.mode_changed())
        ttk.Label(api, text="Model").grid(row=1, column=0, sticky="w", pady=3)
        self.model_entry = ttk.Entry(api, textvariable=self.model)
        self.model_entry.grid(row=1, column=1, sticky="ew", padx=(10, 0), pady=3)
        ttk.Label(api, text="API key").grid(row=2, column=0, sticky="w", pady=3)
        self.key_entry = ttk.Entry(api, textvariable=self.key, show="•")
        self.key_entry.grid(row=2, column=1, sticky="ew", padx=(10, 0), pady=3)
        saved = install.KEY_FILE.exists()
        self.key_hint = ttk.Label(api, text="Leave empty to keep the key you saved before." if saved else "",
                                  style="Hint.TLabel")
        self.key_hint.grid(row=3, column=1, sticky="w", padx=(10, 0))
        self.api_widgets = [self.provider_box, self.model_entry, self.key_entry]

        self.choose_error = ttk.Label(f, text="", style="Error.TLabel", wraplength=500)
        self.choose_error.grid(row=7, column=0, sticky="w", pady=(10, 0))
        f.rowconfigure(8, weight=1)
        buttons = ttk.Frame(f)
        buttons.grid(row=9, column=0, sticky="e")
        ttk.Button(buttons, text="Set up", command=self.start, default="active").pack(side="right")
        self.root.bind("<Return>", lambda e: self.start() if self.pages["choose"].winfo_ismapped() else None)
        self.mode_changed()

    def mode_changed(self):
        on = self.mode.get() == "api"
        for w in self.api_widgets:
            w.state(["!disabled"] if on else ["disabled"])
        if on:
            self.provider_box.state(["readonly"])
        anthropic = PROVIDERS[self.provider.get()] == "anthropic"
        if anthropic:
            self.model.set(install.HAIKU)
            self.model_entry.state(["disabled"])
        elif self.model.get() == install.HAIKU:
            self.model.set("")

    def start(self):
        api = model = key = None
        if self.mode.get() == "api":
            api, model, key = PROVIDERS[self.provider.get()], self.model.get().strip(), self.key.get().strip()
            if not model:
                return self.choose_error.config(text="Enter the provider's model name, e.g. from its docs.")
            if not key and not install.KEY_FILE.exists():
                return self.choose_error.config(text="Paste your API key.")
        self.choose_error.config(text="")
        self.log.delete("1.0", "end")
        self.status.config(text="Starting…")
        self.error.config(text="")
        self.back.grid_remove()
        self.bar.config(mode="indeterminate")
        self.bar.start(12)
        self.choice = (self.provider.get() if api else None, model)
        self.show("progress")
        threading.Thread(target=run_install, args=(self.q, api, model, key), daemon=True).start()

    # --- 2: installing ---------------------------------------------------------------

    def progress_page(self):
        f = self.page("progress", "Setting up localflow")
        self.status = ttk.Label(f, text="", wraplength=500)
        self.status.grid(row=1, column=0, sticky="w", pady=(10, 6))
        self.bar = ttk.Progressbar(f, mode="indeterminate", maximum=1000)
        self.bar.grid(row=2, column=0, sticky="ew")
        self.error = ttk.Label(f, text="", style="Error.TLabel", wraplength=500)
        self.error.grid(row=3, column=0, sticky="w", pady=(8, 0))
        ttk.Label(f, text="Details", style="Hint.TLabel").grid(row=4, column=0, sticky="w", pady=(12, 2))
        box = ttk.Frame(f)
        box.grid(row=5, column=0, sticky="nsew")
        box.columnconfigure(0, weight=1)
        box.rowconfigure(0, weight=1)
        f.rowconfigure(5, weight=1)
        self.log = tk.Text(box, height=10, wrap="word", relief="flat", borderwidth=0,
                           font=("TkFixedFont", max(self.font.actual("size") - 1, 8)), background="#f4f4f4",
                           foreground="#333", padx=8, pady=6)
        scroll = ttk.Scrollbar(box, command=self.log.yview)
        self.log.config(yscrollcommand=scroll.set)
        self.log.grid(row=0, column=0, sticky="nsew")
        scroll.grid(row=0, column=1, sticky="ns")
        self.back = ttk.Button(f, text="Back", command=lambda: self.show("choose"))
        self.back.grid(row=6, column=0, sticky="e", pady=(12, 0))

    def poll(self):
        try:
            while True:
                msg = self.q.get_nowait()
                if msg[0] == "line":
                    line = msg[1]
                    self.log.insert("end", line + "\n")
                    self.log.see("end")
                    if line.startswith("==> "):
                        self.status.config(text=line[4:])
                        self.bar.config(mode="indeterminate")
                        self.bar.start(12)
                elif msg[0] == "progress":
                    done, total = msg[1:]
                    if total:
                        if str(self.bar.cget("mode")) != "determinate":
                            self.bar.stop()
                            self.bar.config(mode="determinate")
                        self.bar.config(value=done * 1000 // total)
                elif msg[0] == "error":
                    self.bar.stop()
                    self.bar.config(mode="determinate", value=0)
                    self.error.config(text=msg[1])
                    self.back.grid()
                elif msg[0] == "done":
                    self.bar.stop()
                    self.finish(msg[1])
        except queue.Empty:
            pass
        self.root.after(50, self.poll)

    # --- 3: how to dictate --------------------------------------------------------------

    def done_page(self):
        f = self.page("done", "You're set")
        self.how = ttk.Label(f, text="", wraplength=500, justify="left")
        self.how.grid(row=1, column=0, sticky="w", pady=(10, 0))
        self.extra = ttk.Frame(f)
        self.extra.grid(row=2, column=0, sticky="ew", pady=(12, 0))
        f.rowconfigure(3, weight=1)
        ttk.Button(f, text="Done", command=self.root.destroy, default="active").grid(row=4, column=0, sticky="e")

    def finish(self, model):
        provider = self.choice[0]
        where = f"{model} with your {provider.split(' (')[0]} key" if provider else f"{model} on this computer"
        keys = KEYS.get(SYSTEM)
        if keys:
            text = (f"Hold {keys[0]} and talk; let go and it pastes. Or tap {keys[0]}, talk, and tap it "
                    f"again. Esc cancels.\n\nFixed a word it got wrong? Select your fix and press {keys[1]}, "
                    f"and localflow spells it right from then on.\n\nCleanup: {where}. localflow starts when "
                    "you sign in; open it again to change these settings.")
        else:
            cmd = " ".join(install.dictate_cmd())
            text = (f"Add keyboard shortcuts in your desktop's settings: `{cmd} toggle` to start and stop "
                    f"(or `{cmd} start` on press and `{cmd} stop` on release), and `{cmd} learn` to learn "
                    f"from a fix.\n\nCleanup: {where}.")
        self.how.config(text=text)
        for w in self.extra.winfo_children():
            w.destroy()
        if SYSTEM == "Darwin":
            ttk.Label(self.extra, text="macOS asks to allow localflow under Accessibility (for the hotkey "
                                       "and pasting) and Microphone.", wraplength=500, justify="left"
                      ).pack(anchor="w")
            ttk.Button(self.extra, text="Open Accessibility settings", command=lambda: os.system(
                "open 'x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility'")
            ).pack(anchor="w", pady=(6, 0))
        self.show("done")


def main():
    if SYSTEM == "Windows":
        try:
            import ctypes
            ctypes.windll.shcore.SetProcessDpiAwareness(1)
        except (AttributeError, OSError):
            pass
    root = tk.Tk()
    icon = Path(getattr(sys, "_MEIPASS", Path(__file__).resolve().parent)) / "icon.png"
    if icon.exists():
        root.iconphoto(True, tk.PhotoImage(file=str(icon)))
    Setup(root)
    root.lift()
    root.focus_force()
    root.mainloop()


if __name__ == "__main__":
    main()
