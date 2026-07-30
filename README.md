<img width="443" height="254" alt="LogoYuna" src="https://github.com/user-attachments/assets/60491faf-f06f-47e9-9ab3-c8a490efe5fd" />

# ForzaLife
Safe add-on for Forza Horizon 6 adding fuel usage, periodic oil maintenance, boost gauge and more.

## Linux companion

This fork adds a native Rust Linux port under [`linux/`](linux/). It listens to the FH6 Data Out UDP feed and renders a transparent, click-through HUD inside gamescope, matching the Windows application's behavior and presentation.

Build and install:

```sh
cd linux
cargo build --release --locked
install -Dm755 target/release/forzalife ~/.local/bin/forzalife
```

Keep the Steam launch option as gamescope only. The enabled `forzalife.service` watcher starts exactly one overlay when FH6 appears:

```sh
gamescope -f -r 144 -w 1920 -h 1080 -- %command%
```

Do not add `forzalife session` to the launch option, or it will compete with the watcher.

Then enable Data Out in FH6 with IP `127.0.0.1` and port `8080`. Set another port by placing `FORZALIFE_PORT=9000` before `gamescope`.

The Linux HUD shows boost with a PSI readout, persistent per-car fuel, odometer, average MPG and km/L, and oil state, plus service navigation using the Windows release's bundled world map. It also provides a driving HUD with speed, gear, RPM, shift stages, throttle, race position, session distance, and Life, Drive, and Minimal modes. Use the arrow keys: Right opens/selects, Up and Down navigate, and Left goes back. Enter confirms numeric input and Escape cancels it. The menu includes mini navigation, the vehicle info card, manual odometer sync, manual fuel level for testing, per-car fuel-usage pause, driving-session reset, HUD mode cycling, and a `Reload overlay` action that reloads the overlay without restarting Forza. Stop within 25 m of a gas station and hold Enter or controller B to refuel gradually with visible progress; a workshop services its oil. Running out of fuel pulse-limits keyboard `W` and the controller right trigger during free roam while forwarding every other input unchanged. The input proxy requires read access to `/dev/input/event*` and write access to `/dev/uinput`; device grabs are released automatically when ForzaLife exits. It does not read memory from or modify FH6. See [`docs/research/linux-overlay.md`](docs/research/linux-overlay.md) for Linux overlay setup notes.

## Hello

Welcome to ForzaLife, an add-on for Forza Horizon 6. The goal is to add some features and scenarios to the game in a safe and non-invasive manner. I use only official telemetry data and don't read or write to any files or memory of the game. The only thing I input in the game is simulating fuel starvation by simulating brake key presses. As it is used only outside of races and in no way can give you an advantage, it complies with the Forza Code of Conduct.

Any feedback is appreciated. If you see bugs, bad translations and any other issues, please report them to me.
Remeber - it's a beta. I has bugs for sure. But together we can squiah them!

If you feel generous you can buy me a beer - https://buymeacoffee.com/puffinflight

## What it does?
ForzaLife currently adds these features:
* **Fuel consumption** for all cars with HUD gauge and **fuel starvation simulation**.
* Ability to **fill up** at fuel stations.
* **Oil maintenance** and car workshops to provide them.
* **Odometer** for every car.
* **Boost gauge** for turbo and supercharged vehicles.
* **Simple navigation** to the nearest gas station or workshop.
* Vehicle info card where you can check next oil maintenance or the persistent trip odometer.
* Many settings to fit your style of play.

## How to use?

* Unpack the add-on and run ForzaLife.exe.
* In Forza go to Settings > HUD & Gameplay, scroll all the way down. Switch on "Data Out".
* Go into the setting of the add-on by double or right clicking on the pink H icon in tray. 
* Set telemetry port to the same port you have in Forza "Data Out IP Port"
* Set brake pedal key the same as your key in Forza (Settings > Controls > Change Input Mapping > Keyboard)
* In game you can invoke forzaLife menu by clicking main interaction key (default: L) and navigate up and down (default semicolon and quote keys)


## FAQ

#### **Why don't the fuel gauge and odometer in my car show the same thing as the add-on HUD?**
Because for an add-on to be safe, it cannot modify Forza's memory or files. Forza does not allow ANY data input into the game.
#### **I have two lovely Nissan Figaros, but the odometer and fuel level are the same for both.**
Unfortunately, I cannot tell the difference between cars of the same model using data from Forza. I am still trying to figure out how to fix this.
#### **Why don't you just use the fuel level from telemetry?**
It only works with the damage simulation switched on, and in Forza Horizon, that is just annoying. The same goes for tire wear. Most players prefer to play with no damage or visual damage only, and these options keep the fuel always at 100%.
#### **One of the gas station attendants is rude!**
It's Reina. We tried. She's just an a-hole.

## Legal Stuff

### 1. Fair Play & Anti-Cheat Compliance
ForzaLife is a **100% safe and compliant** companion application. It operates strictly by listening to the official **UDP Telemetry "Data Out"** feature built directly into the game options by the developers. 
* This application **does not** modify game files.
* This application **does not** inject code into the game process (`.exe`).
* This application **does not** read or write directly to the game's system memory (RAM).
* This application **does** simulate brake key to simulate breakdowns and fuel starvation and is not giving advantage in any way. Because of that it sitll should be compliant with Forza Code of Conduct. Input simulation is disabled in races. 

### 2. Trademark Disclaimer
ForzaLife.Overlay is an unofficial fan-made project and is **not** affiliated, associated, authorized, endorsed by, or in any way officially connected with Microsoft Corporation, Playground Games, Turn 10 Studios, or any of their subsidiaries or affiliates. 

The official "Forza", "Forza Horizon", and "Forza Motorsport" names, as well as related marks, emblems, logos, and images, are registered trademarks of Microsoft Corporation and their respective owners. Brand names and logos used within the vehicle data profiles are for simulation and immersion purposes only.


## Known and unconfirmed issues

* I'm not a native English speaker, nor Japanese. If you see any errors in dialogues and other texts, please let me know.
* Currently only keyboard and XInput gamepads are supported. If you can test wheel inputs, let me know if it worked. I can't do it myself currently.
* Add-on is only compatible with game in fullscreen mode and with analog HUD. If there is a demand I can add digital and no HUD options.
* Add-on was not tested on ultrawide monitors yet. Report if you have any issues, preferably with a screenshot.
* If the fuel gauge is not visible please report with a screenshot, resolution of your screen AND resolution set in the game.
* Currently fuel use is active in the eliminator and hide and seek modes. I'm working on detecting them, but it's tricky. The data from telemetry in case of race status is not reliable at all.

## AI Disclaimer

The ForzaLife add-on was not vibe-coded. AI was use to generate character illustrations. If you're against AI use you can remove those from the assets folder. The add-on should still work.

## Assets used
* CC0 sounds from freesound.org by BigDino1995, collierhs_colinlib, sevenbsb, XiiiSamples
