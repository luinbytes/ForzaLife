# ForzaLife
Safe add-on for Forza Horizon 6 adding fuel usage, periodic oil maintenance, boost gauge and more.

## Hello

Welcome to ForzaLife, an add-on for Forza Horizon 6. The goal is to add some features and scenarios to the game in a safe and non-invasive manner. I use only official telemetry data and don't read or write to any files or memory of the game. The only thing I input in the game is simulating fuel starvation by simulating brake key presses. As it is used only outside of races and in no way can give you an advantage, it complies with the Forza Code of Conduct.

Any feedback is appreciated. If you see bugs, bad translations and any other issues, please report them to me.

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
