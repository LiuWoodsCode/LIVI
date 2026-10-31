# LIVI integration API

This document describes how another program can supply vehicle data to LIVI, receive its state, and control it.
It documents the interfaces implemented in this checkout. Run LIVI before connecting; the telemetry CLI sends to an
existing instance and does not start one.

## Choose an interface

| Need | Interface | Address |
| --- | --- | --- |
| Supply vehicle readings from any language | HTTP POST with JSON | `http://<livi-host>:4000/telemetry` |
| Supply readings and receive the merged telemetry live | Socket.IO | `http://<livi-host>:4000`, namespace `/`, path `/socket.io/` |
| Send readings from a shell | Telemetry CLI | `livi-core telemetry ...` |
| Read all application state, control media/devices/screens, change settings, or send input | Framed JSON over a local Unix socket | Runtime directory's `core.sock` |
| Read status, media, navigation, GPS, or climate without a socket client | JSON state files | LIVI data directory's `state/` folder |
| Connect a vehicle microcontroller | USB serial bridge | Auto-detected USB product `LIVI Bridge` |

The network telemetry API carries vehicle readings and a few commands such as page selection and volume. It does not
expose every local control action. Socket.IO publishes telemetry, not the entire application state: use local IPC or
state files for now-playing metadata and navigation from the phone. Writing arbitrary fields such as `nowPlaying` into
telemetry does not update the corresponding application state.

The telemetry server listens on port **4000**, first trying all IPv6 interfaces, then all IPv4 interfaces. It has
permissive CORS, no authentication, and no built-in TLS. Keep access on a trusted network or provide access controls
outside LIVI. Local IPC checks the connecting process's user ID against the core's user ID.

## Send telemetry with HTTP

Send a JSON object containing the fields that changed:

```bash
curl --fail-with-body \
  -H 'Content-Type: application/json' \
  --data '{"speedKph":50,"rpm":1500,"gear":"D","lights":true}' \
  http://127.0.0.1:4000/telemetry
```

Replace `127.0.0.1` with the host running LIVI for remote integrations. `livi.local` works only if that hostname resolves
on your network.

| Result | Meaning |
| --- | --- |
| `204 No Content` | The body is a JSON object and was submitted to the telemetry queue. There is no response body. |
| `400 Bad Request` | The body is malformed JSON or is not an object, for example an array, string, or `null`. |

An empty object is accepted but makes no change. The handler reads raw bytes, so JSON is accepted even without the
content-type header; sending the header is recommended. Framework-level request limits can reject oversized bodies.

A `204` does not confirm that a phone or vehicle applied a value. The ingress handler does not validate the full
telemetry schema. Unknown fields are retained and broadcast; a wrong type can be stored while being ignored by its
consumer. Validate sensor data in the producer, and subscribe to updates if you need to observe the merged state.
There is no HTTP GET endpoint for reading the telemetry snapshot.

This Python example uses only the standard library:

```python
import json
from urllib.request import Request, urlopen


def push_telemetry(patch, host="127.0.0.1:4000"):
    request = Request(
        f"http://{host}/telemetry",
        data=json.dumps(patch, allow_nan=False).encode("utf-8"),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    with urlopen(request, timeout=5) as response:
        if response.status != 204:
            raise RuntimeError(f"Unexpected HTTP status: {response.status}")


push_telemetry({"speedKph": 50, "gear": "D"})
push_telemetry({"gps": {"lat": 53.5912, "lng": 10.015, "speedMs": 13.89}})
```

## Send and receive telemetry with Socket.IO

Use a Socket.IO client compatible with Engine.IO v4, such as `socket.io-client` v4. This endpoint is **Socket.IO**, not a
plain WebSocket that accepts bare JSON messages.

| Direction | Event | Payload |
| --- | --- | --- |
| Client → LIVI | `telemetry:push` | A JSON object with a partial telemetry update |
| LIVI → clients | `telemetry:update` | The complete merged telemetry snapshot |

On connection to namespace `/`, LIVI sends the current snapshot if it is nonempty. Each nonempty object push merges
into the store and broadcasts the resulting snapshot to every connected client, including the producer. There is no
application-level acknowledgement callback for `telemetry:push` and no per-producer ownership or subscription filter.

For Node.js, install the client in your integration project:

```bash
npm install socket.io-client@4
```

Save this as `telemetry-client.mjs` and run it with `node telemetry-client.mjs`:

```javascript
import { io } from "socket.io-client";

const socket = io("http://127.0.0.1:4000", {
  path: "/socket.io/",
});

socket.on("telemetry:update", (snapshot) => {
  // Each event replaces the client's telemetry snapshot.
  console.log("speed:", snapshot.speedKph, "climate:", snapshot.climate);
});

socket.on("connect", () => {
  // Runs again after reconnect: publish current readings here.
  socket.emit("telemetry:push", {
    speedKph: 50,
    gear: "D",
    gps: { lat: 53.5912, lng: 10.015 },
  });
});

socket.on("connect_error", (error) => console.error(error.message));
socket.on("disconnect", (reason) => console.log("disconnected:", reason));
```

Register listeners before connecting or emitting data. After a reconnect, the initial update reflects LIVI's current
store; send fresh sensor readings instead of replaying an old queue of vehicle states.

## Telemetry merge rules

HTTP, Socket.IO, the CLI, the serial bridge, and LIVI's internal producers feed the same telemetry store.

1. A push can contain any subset of fields. Omitted fields keep their previous values.
2. `gps`, `can`, `climate`, `tirePressure`, and `notification` merge one level of object fields. Other values, including
   arrays and other objects, replace their previous value as a whole.
3. `null` is stored as a value; it does not remove a key. Whether a consumer treats it as unavailable or falls back to
   another field depends on that consumer. For example, `clusterNightMode: null` resumes following `nightMode`.
4. Every nonempty push gets a `ts` in Unix milliseconds unless it supplies one. A supplied `ts` is stored as given;
   it does not provide ordering, conflict resolution, or rejection of old data.
5. The legacy key `turn` becomes `turnSignal`. If both appear in one push, `turnSignal` wins.
6. A climate object without `source` gets `source: "external"` before merging, even if the previous source was
   `carplay`.

For example, these consecutive pushes:

```json
{"gps":{"lat":53.59,"lng":10.01},"gear":"D"}
```

```json
{"gps":{"lat":53.60},"gear":"R"}
```

leave `gps.lng` at `10.01`, update `gps.lat` to `53.60`, and replace `gear` with `"R"`. Updates from different producers
are processed in arrival order, so coordinate ownership of shared fields. LIVI does not expire stale readings or
automatically reset `reverse`, `gear`, or other values when a producer disconnects. Publish transitions explicitly.

## Telemetry field reference

Names are case-sensitive JSON keys. Numbers below are JSON numbers, booleans are `true`/`false`, and units are the
same regardless of the display's unit settings. Send finite numbers; JSON does not support NaN or infinity.

Storing a field does not guarantee that a dashboard widget or a phone uses it. The dashboard currently displays a
subset. The per-consumer support matrix is maintained in
[the telemetry contract](native/crates/livi-core-proto/src/telemetry.rs); projection adapters live in
[aa_telemetry.rs](native/crates/livi-core/src/aa_telemetry.rs) and
[cp_telemetry.rs](native/crates/livi-core/src/cp_telemetry.rs).

### Motion, body, and driving state

| Field | Type / unit | Meaning |
| --- | --- | --- |
| `ts` | Number, Unix ms | Snapshot update time; filled by LIVI when omitted. |
| `speedKph` | Number, km/h | Vehicle speed. |
| `rpm` | Number, revolutions/min | Engine speed. |
| `gear` | Number or string | Manual: `-1` reverse, `0` neutral, `1`–`10` forward. Automatic: `P`, `R`, `N`, `D`, `S`, `M1`–`M10`. |
| `reverse` | Boolean | Explicit reverse state; takes precedence over `gear`. Clear it when leaving reverse. |
| `steeringDeg` | Number, degrees | Steering angle; negative means left. |
| `lights` | Boolean | Low beam. |
| `highBeam` | Boolean | High beam; Android Auto uses this even if `lights` is false. |
| `hazards` | Boolean | Hazard warning lights. |
| `turnSignal` | String | `"none"`, `"left"`, or `"right"`; legacy alias `turn` is accepted. |
| `parkingBrake` | Boolean | Parking brake state. |
| `odometerKm` | Number, km | Total distance; Android Auto receives tenths of a kilometre. |
| `odometerTripKm` | Number, km | Trip distance. |
| `drivingStatus` | Unsigned 32-bit integer | Android Auto driving-restriction bit field; `0` means no restrictions. |

When `autoSwitchOnReverse` is enabled, `gear`/`reverse` can bring the reverse camera to the front of the main screen.
CarPlay also derives limited-UI and parked-video availability from gear, so send real driving state.

### Engine, environment, fuel, and electrical readings

| Field | Type / unit | Meaning |
| --- | --- | --- |
| `coolantC`, `oilC`, `transmissionC`, `iatC` | Number, °C | Coolant, engine oil, transmission oil, and intake-air temperature. |
| `ambientC` | Number, °C | Outside temperature; also forwarded to Android Auto and CarPlay. |
| `batteryV` | Number, V | The vehicle's low-voltage battery, typically 12 V. |
| `fuelPct` | Number, 0–100 | Fuel level, or state of charge for an electric vehicle. |
| `rangeKm` | Number, km | Remaining range. |
| `rangeFuelKm`, `rangeElectricKm` | Number, km | Separate fuel and electric ranges for hybrids. |
| `fuelRateLph` | Number, L/h | Instantaneous fuel rate. |
| `consumptionLPer100Km` | Number, L/100 km | Instantaneous consumption. |
| `consumptionAvgLPer100Km` | Number, L/100 km | Average consumption. |
| `mapKpa` | Number, kPa | Manifold absolute pressure. |
| `baroKpa` | Number, kPa | Atmospheric pressure; also sent to Android Auto. |
| `boostKpa` | Number, kPa | Boost pressure. |
| `throttlePct` | Number, 0–100 | Throttle position. |
| `lambda` | Number | Lambda; `1.0` is stoichiometric. |
| `afr` | Number | Air-to-fuel ratio. |
| `ambientLux` | Number, lux | Ambient illuminance. |
| `dimmerPct` | Number, 0–100 | Instrument-light dimmer. |

### Electric vehicle capabilities and state

| Field | Type / unit | Meaning |
| --- | --- | --- |
| `batteryCapacityKwh` | Number, kWh | Gross battery capacity. A positive capacity enables electric routing in Android Auto regardless of `carType`. |
| `batteryLevelKwh` | Number, kWh | Current energy; adapters can derive this from `fuelPct` and capacity when absent. |
| `charging` | Boolean | True while charging. |
| `activeConnector` | String | Connector currently charging, for example `"Combo2"`. |
| `rangeMaxElectricKm` | Number, km | Electric range with a full battery. |
| `carType` | String | Vehicle type name; recognized values also update persistent settings. |
| `evConnectorTypes` | Array of strings | Supported charging connectors; recognized values also update persistent settings. |
| `evMaxAcPowerKw`, `evMaxDcPowerKw` | Number, kW | Maximum accepted charging power; positive values also update persistent settings. |

Accepted telemetry `carType` names, ignoring case and surrounding whitespace:
`Gasoline`, `DieselWinter`, `Diesel`, `Biodiesel`, `E85`, `LPG`, `CNG`, `LNG`, `Electric`, `Hydrogen`, `Other`,
`HybridGasoline`, `HybridDiesel`.

Accepted telemetry `evConnectorTypes` names, also ignoring case and surrounding whitespace:
`J1772`, `Mennekes`, `Chademo`, `Combo1`, `Combo2`, `TeslaSupercharger`, `Gbt`, `Other`.
Unrecognized names are skipped when updating settings, and an empty recognized list leaves settings unchanged.
`"Type 2"` is not an alias for `"Mennekes"`. For `activeConnector`, use the exact spelling listed above; CarPlay's
active-connector adapter currently maps all of these except `Other`.

```json
{
  "carType": "Electric",
  "batteryCapacityKwh": 75,
  "batteryLevelKwh": 45,
  "fuelPct": 60,
  "rangeKm": 280,
  "charging": true,
  "activeConnector": "Combo2",
  "evConnectorTypes": ["Mennekes", "Combo2"],
  "evMaxAcPowerKw": 11,
  "evMaxDcPowerKw": 150
}
```

### GPS, tires, CAN, and receiver diagnostics

`gps` merges by field:

| GPS field | Type / unit | Meaning |
| --- | --- | --- |
| `lat`, `lng` | Number, decimal degrees | Latitude and longitude. |
| `alt` | Number, m | Altitude. |
| `heading` | Number, degrees | Course heading. |
| `speedMs` | Number, m/s | GPS speed; this uses different units from top-level `speedKph`. |
| `accuracyM` | Number, m | Position accuracy. |
| `satellites` | Number | Satellite count. |
| `fixTs` | Number, Unix ms | Time of the fix. |

Push coherent latitude/longitude pairs together to avoid combining coordinates from different fixes. LIVI can also
populate GPS through its configured serial GNSS receiver. If an external program owns GPS, coordinate it with that
receiver to avoid competing updates.

`tirePressure` contains numeric kPa fields `frontLeftKpa`, `frontRightKpa`, `rearLeftKpa`, and `rearRightKpa`. A producer
can update one wheel at a time.

`can` contains `id` (unsigned 32-bit integer), `data` (array of bytes, each 0–255), and optional `bus` (0–255):

```json
{"can":{"id":291,"data":[1,2,255],"bus":0},"tirePressure":{"frontLeftKpa":230}}
```

The `can` block is diagnostic data. It does not decode CAN frames or transmit them to a CAN bus. A CAN/OBD integration
must decode its signals and push fields such as `speedKph` and `rpm` itself. Because `can` merges by field, send all
frame fields together when publishing a new frame.

`gnss` is LIVI's own receiver-diagnostic object, replaced as a whole. It includes connection state, fix quality/mode,
satellite details, dilution of precision, receiver identity, RF information, time, and timezone. External position
producers should normally use `gps`. The complete diagnostic shape is defined in
[gnss.rs](native/crates/livi-core-proto/src/gnss.rs).

### Appearance, volume, and page selection

| Field | Type | Behavior |
| --- | --- | --- |
| `nightMode` | Boolean | Day/night for Android Auto and CarPlay. |
| `clusterNightMode` | Boolean or null | Separate CarPlay cluster appearance; absent/null follows `nightMode`. |
| `darkMode` | Boolean | LIVI's own dark/light appearance; also updates its persistent Dark Mode setting. |
| `volume` | Number, 0–1 | Sets persistent `huVolume`, which drives the amplifier and linked system mixer. |
| `path` | String | Requests a main-screen page or a projected phone app. |

The appearance setting (`auto`, `day`, `night`) and LIVI's own `darkMode` are separate. Selecting `day` or `night`
through settings updates `nightMode` and clears `clusterNightMode`. Volume is clamped to 0–1 for the setting, and
changes smaller than `0.005` from the last applied level are ignored for that setting; the raw submitted value is
still stored in telemetry. Producers should send values already in range.

Supported main-screen paths include `/projection`, `/telemetry` (dashboard), `/media`, `/media/phone`,
`/media/dabplus`, `/camera`, `/settings`, and valid settings subpaths such as `/settings/devices`.
A non-projection page request is sent to the UI when the stored path changes. Repeating the same stored path does not
reopen it, even if the user has navigated elsewhere. Unknown page names are ignored by the UI.

`/projection` brings projection forward on every push. A suffix requests an app in the active session:

```bash
curl --fail-with-body -d '{"path":"/projection/maps"}' http://127.0.0.1:4000/telemetry
curl --fail-with-body -d '{"path":"/projection/mobilephone"}' http://127.0.0.1:4000/telemetry
```

CarPlay uses a request-UI URL; a bare app name gets `:` appended, and a full URL can be passed, for example
`/projection/comgooglemaps://?daddr=Hamburg`. Android Auto maps known categories such as `maps`, `nowplaying`, `radio`,
and `mobilephone` to its app-selection keys. The phone and installed apps determine which requests work. An app
request requires an active session; a UI must be connected to handle main-screen page changes.

### Climate: bidirectional vehicle integration

The `climate` block merges by field and reaches CarPlay Ultra's climate controls. Changes made on the phone return
through the same telemetry store. An external HVAC controller can subscribe to `telemetry:update`, compare the
climate block against its previous value, apply changes to its hardware, and report actual state back.

| Climate field | Type / range | Meaning |
| --- | --- | --- |
| `source` | String | Origin marker; see below. |
| `on`, `auto`, `ac` | Boolean | System power, automatic mode, air conditioning. |
| `maxAc`, `maxDefrost`, `recirculation` | Boolean | Maximum A/C, maximum defrost, recirculation. |
| `zonesSynced` | Boolean | Passenger target follows driver target. |
| `driverTargetC`, `passengerTargetC` | Number, °C | Targets: 16–28 in 0.5-degree steps; ends display as LO/HI. |
| `driverCabinC`, `passengerCabinC` | Number, °C | Measured cabin temperatures. |
| `fanLevel` | Integer, 0–7 | Fan level; 0 is off. |
| `fanAuto` | Boolean | Automatic fan control. |
| `frontWindowHeating`, `rearWindowHeating` | Integer, 0 or 1 | Window heating. |
| `ventUpper`, `ventMiddle`, `ventLower` | Boolean | Enabled vent outlets. |
| `ventsAuto` | Boolean | Automatic vent selection. |
| `driverSeatHeating`, `passengerSeatHeating` | Integer, 0–2 | Seat-heating levels. |

LIVI uses these source markers:

| `climate.source` | Meaning |
| --- | --- |
| `carplay` | A change from the phone. The CarPlay adapter avoids echoing it straight back. |
| `external` | A push with no explicit source. |
| `saved` | Climate restored from the previous run. |
| `default` | Initial built-in climate. |

An external controller can use its own marker, such as `"car"`; sources other than `carplay` can reach the phone.
Do not label external sensor changes `carplay`. Act only on relevant changes, rather than echoing every received
snapshot, to avoid feedback loops. Socket.IO has no separate HVAC-command acknowledgement: report the resulting
hardware state as another partial push.

```json
{"climate":{"source":"car","on":true,"driverTargetC":22,"fanLevel":3}}
```

CarPlay climate behavior includes zone synchronization, manual fan changes ending AUTO, and MAX A/C restoring prior
settings when it ends. Climate persists across LIVI restarts; an integration should report fresh hardware state when
it connects.

### CarPlay Ultra notifications

`notification` merges `title` (string), `text` (string), `symbol` (string), and `open` (boolean). Push `open: true` to
show a notification and `open: false` to close it:

```bash
curl --fail-with-body -H 'Content-Type: application/json' \
  -d '{"notification":{"title":"Frost","text":"Icy roads","symbol":"snowflake","open":true}}' \
  http://127.0.0.1:4000/telemetry

curl --fail-with-body -d '{"notification":{"open":false}}' http://127.0.0.1:4000/telemetry
```

`symbol` can be an SF Symbol such as `snowflake`, or the basename of a PNG in `~/.config/LIVI/carplay-images`, for example
`example` for `example.png`. Custom images require the phone to fetch the image folder at a subsequent CarPlay start
and in practice require a theme. SF Symbols work without that custom-image setup. Notifications appear in the
notification center and, with a matching theme, the dashboard; this is not a general LIVI desktop notification API.

## Telemetry CLI

```text
livi-core telemetry set <field>=<value> ... [--every-ms <ms>] [--to <host:port>]
livi-core telemetry demo [--to <host:port>]
livi-core telemetry help
```

The CLI posts to the same HTTP endpoint. Its default target is `127.0.0.1:4000`. It parses numbers, `true`, `false`,
and `null` as typed values; everything else is a string. Dots construct nested objects. Quote arguments containing
spaces. Use HTTP or Socket.IO for arrays and structured JSON; the CLI does not parse a JSON array literal as an array.

```bash
livi-core telemetry set speedKph=50 rpm=1500 gear=D
livi-core telemetry set gps.lat=53.5912 gps.lng=10.015
livi-core telemetry set climate.driverTargetC=22 climate.fanLevel=3
livi-core telemetry set clusterNightMode=null
livi-core telemetry set 'notification.text=Icy roads' notification.open=true
livi-core telemetry set speedKph=90 --every-ms 1000 --to livi.local:4000
```

`--every-ms` must be positive; it repeats the same payload until interrupted, rather than generating changing readings.
`demo` sends a set of plausible demonstration readings. A successful CLI send prints the submitted object; a failed
connection or non-2xx HTTP response returns a failure exit code.

On macOS the bundled executable is `LIVI.app/Contents/Resources/core/livi-core`. Linux AppImages forward the telemetry
subcommand, for example `./LIVI-<version>.AppImage telemetry set gear=R`.

## Local core IPC

### Socket discovery and framing

The core listens at:

- `$XDG_RUNTIME_DIR/livi/core.sock` when `XDG_RUNTIME_DIR` is nonempty.
- Otherwise, `<system-temp-dir>/livi-<uid>/core.sock`. On Linux the temp directory is commonly `/tmp`; macOS often uses
  a per-user directory selected by `TMPDIR`.

The core logs the exact path as `[core] listening on ...`. The Rust client helper honors `LIVI_CORE_SOCKET` as a client
override; this does **not** change where the core binds. Use the same user and runtime environment as the running core.
The peer user ID must match; connecting as root to another user's core is not accepted simply because root can open
the socket.

Each message consists of:

```text
4 bytes: unsigned JSON body length in little-endian order
N bytes: UTF-8 JSON body, without a trailing delimiter
```

The maximum body length is **16 MiB** (`16 << 20`). Reads can contain part of a frame or several frames; buffer and
decode accordingly. This socket accepts neither HTTP nor newline-delimited JSON.

### Handshake and messages

The first frame must be a hello. The protocol version in this checkout is **11**:

```json
{"type":"hello","protocol":11,"client":"my-vehicle-controller"}
```

The core responds with `welcome` containing `protocol`, application `version`, state revision `rev`, and the complete
`state`. A mismatched protocol or a different first message produces `refused` with a `reason` and closes the
connection. Invalid JSON after a successful handshake is logged and skipped; an oversized frame ends the connection.
Use [the protocol constant](native/crates/livi-core-proto/src/lib.rs) from the matching LIVI checkout when building
clients, rather than assuming protocol 11 will remain valid across releases.

| Client message | Purpose |
| --- | --- |
| `{"type":"resync"}` | Request a fresh `welcome` snapshot. |
| `{"type":"action","id":1,"action":{...}}` | Run a control action; receive `reply` with the same unsigned 32-bit ID. |
| `{"type":"input","input":{...}}` | Send input; no reply. |
| `{"type":"path","path":"/media"}` | Report the UI's current path for status tracking. This is not a page-opening command. |
| `{"type":"spectrum","on":true}` | Subscribe this connection to audio-spectrum frames; send false to stop. |
| `{"type":"linkSpeed","on":true}` | Enable link-speed monitoring while this connection is subscribed; read `state.system.linkSpeed`. |

| Core message | Purpose |
| --- | --- |
| `welcome` | Full snapshot, including after resync or a server-side lag recovery. Replace local state. |
| `patch` | Next state revision with ordered `ops`. |
| `reply` | Action ID, plus `error` only on failure. Success looks like `{"type":"reply","id":1}`. |
| `refused` | Handshake failure with `reason`. |
| `spectrum` | `bands`, an array of floating-point audio-spectrum values. |
| `openPage` | `path` the connected UI should open. |

Unsolicited state patches can arrive between an action and its reply. Correlate by `id`, not by message position.
A successful reply means the action handler accepted the request; many actions queue work, and a missing session or
unsupported operation can still result in no visible effect. Observe state changes and operation-specific errors for
completion. There is no local IPC action to push telemetry; use HTTP or Socket.IO for that.

### State and incremental updates

| State key | Contents |
| --- | --- |
| `front` | Current content for each of `main`, `cluster`, `aux`. |
| `sessions` | Active protocol (`carplay`, `androidauto`, or null), `position`, and `total`. |
| `devices` | Device IDs/names, `active`/`available`/`offline` status, protocol, transport, and available phone readings. |
| `nowPlaying` | `title`, `artist`, `album`, `app`, `durationMs`, `elapsedMs`, `playing`, `artwork`. |
| `navigation` | Guidance activity, roads, destination, maneuver details, distance, time, ETA, images, and lane guidance. |
| `telemetry` | The same merged telemetry object published over Socket.IO. |
| `system` | Available radios, displays, audio devices, fonts, and optional link-speed readings. |
| `config` | Current complete configuration. |
| `cluster` | Cluster theme/layout state; use the IDs advertised here for layout actions. |
| `dab` | Receiver, stations, scan progress, playing station, radio text, signal, and errors. |
| `cameras` | Available camera IDs and names. |
| `mfi` | Imported authentication-file availability, generation, and errors. |
| `update` | Release availability, progress, phase, and errors. |

Fields not reported by a phone may be null or omitted. Media durations and positions use milliseconds; navigation
numeric distances use metres, remaining times use seconds, and numeric ETA uses Unix seconds. Prefer provided display
strings for formatted guidance. Full shapes are defined in
[state.rs](native/crates/livi-core-proto/src/state.rs) and [theme.rs](native/crates/livi-core-proto/src/theme.rs).

Patches use LIVI's own operations, not RFC 6902 JSON Patch:

```json
{
  "type": "patch",
  "rev": 8,
  "ops": [
    {"op":"set","path":["nowPlaying","title"],"value":"New song"},
    {"op":"remove","path":["telemetry","customField"]}
  ]
}
```

Apply operations in order. A path is an array of object-key strings. `set` inserts or replaces the value; an empty
path replaces the root. `remove` deletes an existing member and cannot remove the root. Parent objects must already
exist. Arrays are replaced whole rather than edited by numeric paths. Revisions increase when state changes; if a
revision is missed or patch application fails, send `resync` and replace state from the next `welcome`. A client must
also accept a fresh `welcome` without requesting one when it falls behind the core's patch backlog.

### Control action reference

Wrap every action below in `{"type":"action","id":<u32>,"action":<object>}`. The `kind` names and enum strings are
case-sensitive.

| Action `kind` | Additional fields | Behavior |
| --- | --- | --- |
| `setConfig` | `patch`: object | Merge and persist configuration. |
| `applySettings` | None | Apply pending settings, including changes needing projection reconfiguration. |
| `show` | `screen`, `front` | Select content on a screen. Screens: `main`, `cluster`, `aux`; fronts: `livi`, `projection`, `cluster`, `camera`. |
| `media` | `control` | `playPause`, `play`, `pause`, `next`, `prev`; routed to the relevant phone/DAB media source. |
| `seek` | `positionMs`: u32 | Request media position; currently implemented for the active CarPlay session. |
| `selectDevice` | `id`: string | Bring a connected device's session forward. |
| `nextDevice` | None | Cycle sessions. |
| `connectDevice` | `id`: string | Ask to connect a paired device. |
| `forgetDevice` | `id`: string | End/forget a device pairing. |
| `selectClusterLayout` | `layout`: string | Select an advertised cluster layout ID. |
| `stepClusterLayout` | `by`: signed 32-bit integer | Step through layouts, for example `1` or `-1`. |
| `moveClusterLayout` | `layout`: string, `by`: signed 32-bit integer | Reorder a layout. |
| `showClusterLayout` | `layout`: string, `on`: boolean | Enable/disable a layout. |
| `dabScan`, `dabStopScan` | None | Start/stop a DAB station scan. |
| `dabPlay` | `serviceId`: u32 | Play a service from `dab.stations`. |
| `dabResume`, `dabPause`, `dabStop` | None | Control DAB playback. |
| `dabFavorite` | `serviceId`: u32, `favorite`: boolean | Set favorite state. |
| `dabHide` | `serviceId`: u32, `hidden`: boolean | Set hidden state. |
| `dabMove` | `serviceId`: u32, `by`: signed 32-bit integer | Reorder a station. |
| `setDongleRadio` | `radio`: `wifi` or `bt`, `on`: boolean | Toggle a LIVI Link radio. |
| `importMfi` | `file`: `certificate` or `identity`, `path`: string | Import a file from a path on the core's host. |
| `removeMfi` | `file`: `certificate` or `identity` | Remove an imported file. |
| `checkUpdate`, `downloadUpdate`, `installUpdate`, `abortUpdate` | None | Control the updater; observe `state.update`. |
| `quit`, `restart` | None | Stop or request restarting LIVI. |

Use device IDs, service IDs, and layout IDs from state instead of assuming names are identifiers. Hardware-dependent
actions require the corresponding hardware/service. Updater and DAB actions can return service-unavailable errors.

Example action frames:

```json
{"type":"action","id":1,"action":{"kind":"media","control":"next"}}
```

```json
{"type":"action","id":2,"action":{"kind":"show","screen":"main","front":"projection"}}
```

```json
{"type":"action","id":3,"action":{"kind":"setConfig","patch":{"huVolume":0.5,"dashboard":{"main":true}}}}
```

### Configuration behavior

`setConfig` merges nested objects recursively; arrays/scalars replace their old value. A top-level null removes a
setting before configuration is deserialized: it clears optional fields, while removing a required field returns an
error. For example, `primaryColorDark: null` clears an optional color, but `carName: null` is rejected. This differs
from telemetry null semantics. Invalid types/enum values, unknown non-null top-level settings, and invalid changed
Wi-Fi passwords return an error. Wi-Fi passwords must contain 8–63 characters.

On success the core saves the main configuration file before updating `state.config`. Some changes apply live;
projection-related settings can require `applySettings`, session reconfiguration, or a restart. Follow the settings
schema's behavior for the option you change. The complete schema and defaults are in
[config.rs](native/crates/livi-core-proto/src/config.rs); settings UI schemas are under
[settings/schemas](native/crates/livi-ui-model/src/settings/schemas).

Telemetry capability names and configuration representations differ: telemetry accepts `carType: "Electric"`, while
`setConfig` uses the numeric enum value (`carType: 10`). Likewise, telemetry takes connector names, while the config
stores numeric connector enum values. Consult the configuration enums when patching these settings directly.

### Input messages

Pointer coordinates are normalized to 0–1. Keep touch IDs stable through a gesture and send the corresponding `up`
or `cancel` phase when it ends:

```json
{"type":"input","input":{"kind":"pointer","screen":"main","points":[{"id":0,"x":0.25,"y":0.5,"phase":"down"}]}}
```

```json
{"type":"input","input":{"kind":"pointer","screen":"main","points":[{"id":0,"x":0.25,"y":0.5,"phase":"up"}]}}
```

Phases are `down`, `move`, `up`, `cancel`. Current core handling forwards pointers only for `screen: "main"` while
projection is frontmost; cluster/aux pointer input is ignored. This interface forwards projection input rather than
clicking arbitrary native UI widgets.

Keys use the configured binding's physical-code string, for example:

```json
{"type":"input","input":{"kind":"key","code":"KeyH","down":true}}
```

Send key-up with `down: false` as appropriate. The meaning depends on `config.bindings`, active projection, and whether
the binding works in the background. Use explicit `media` or screen actions when you need a control independent of a
user's key bindings.

### Complete Python IPC client example

This standard-library example discovers the default socket, performs the handshake, applies state patches, and
requests a media action. Set `LIVI_CORE_SOCKET` to the logged socket path if the producer's runtime environment differs
from LIVI's. Run it with the same user as LIVI; stop with Ctrl+C.

```python
import json
import os
import socket
import struct
import tempfile

PROTOCOL = 11  # Match native/crates/livi-core-proto/src/lib.rs.
MAX_FRAME = 16 << 20


def socket_path():
    override = os.environ.get("LIVI_CORE_SOCKET")
    if override:
        return override
    runtime = os.environ.get("XDG_RUNTIME_DIR")
    base = os.path.join(runtime, "livi") if runtime else os.path.join(
        tempfile.gettempdir(), f"livi-{os.getuid()}"
    )
    return os.path.join(base, "core.sock")


def send_frame(connection, message):
    body = json.dumps(message, separators=(",", ":"), allow_nan=False).encode("utf-8")
    if len(body) > MAX_FRAME:
        raise ValueError("Frame exceeds 16 MiB")
    connection.sendall(struct.pack("<I", len(body)) + body)


def read_exact(connection, size):
    result = bytearray()
    while len(result) < size:
        chunk = connection.recv(size - len(result))
        if not chunk:
            raise EOFError("LIVI closed the connection")
        result.extend(chunk)
    return bytes(result)


def receive_frame(connection):
    length, = struct.unpack("<I", read_exact(connection, 4))
    if length > MAX_FRAME:
        raise ValueError("Frame exceeds 16 MiB")
    return json.loads(read_exact(connection, length))


def apply_op(state, operation):
    path = operation["path"]
    if not path:
        if operation["op"] != "set":
            raise ValueError("Cannot remove the root")
        return operation["value"]
    parent = state
    for key in path[:-1]:
        parent = parent[key]
    if operation["op"] == "set":
        parent[path[-1]] = operation["value"]
    elif operation["op"] == "remove":
        del parent[path[-1]]
    else:
        raise ValueError("Unknown patch operation")
    return state


with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
    connection.connect(socket_path())
    send_frame(connection, {"type": "hello", "protocol": PROTOCOL, "client": "example"})
    state, revision = None, None
    requested_action = False
    while True:
        message = receive_frame(connection)
        kind = message["type"]
        if kind == "refused":
            raise RuntimeError(message["reason"])
        if kind == "welcome":
            if message["protocol"] != PROTOCOL:
                raise RuntimeError("Protocol mismatch")
            state, revision = message["state"], message["rev"]
            if not requested_action:
                send_frame(connection, {
                    "type": "action", "id": 1,
                    "action": {"kind": "media", "control": "next"},
                })
                requested_action = True
        elif kind == "patch":
            if revision is None:
                continue  # Await the replacement welcome after a resync.
            try:
                if message["rev"] != revision + 1:
                    raise ValueError("Missed state revision")
                for operation in message["ops"]:
                    state = apply_op(state, operation)
                revision = message["rev"]
            except (KeyError, TypeError, ValueError):
                state, revision = None, None
                send_frame(connection, {"type": "resync"})
                continue
        elif kind == "reply":
            print("action", message["id"], message.get("error", "accepted"))
        elif kind == "openPage":
            print("UI page request:", message["path"])
        if kind in ("welcome", "patch") and state is not None:
            print("now playing:", state.get("nowPlaying"))
```

For a long-running integration, reconnect after EOF with bounded backoff, repeat the handshake, and discard the old
state/revision. A successful reconnect starts from a new `welcome`; there is no replay of actions or missed events.
Avoid automatically replaying actions such as `next`, `forgetDevice`, or `installUpdate` after an uncertain disconnect.

### Rust client library

Integrations inside this Rust workspace can use [livi-core-client](native/crates/livi-core-client/src/lib.rs) with
[livi-core-proto](native/crates/livi-core-proto/src/lib.rs). `socket_path()` discovers the socket;
`wait_for_socket()` waits for it; `Client::start()` handshakes and starts a reader thread. The callback receives the
current JSON state and `Welcome`, `Changed`, `Spectrum`, `OpenPage`, or `Gone` events.

Methods include `act()`, `input()`, `show()`, `path()`, `spectrum()`, `send()`, `state()`, and typed `get()`.
The callback runs while the library holds the state mutex: use the supplied state argument there instead of calling
`state()`/`get()` again. The convenience client currently ignores action replies and does not expose a reply event;
use your own framed connection when acknowledgement/error handling is required. It does not reconnect automatically.

## Read JSON state files

State files live next to `config.json`, under `state/`:

| Platform | Default LIVI data directory |
| --- | --- |
| Linux | `$XDG_CONFIG_HOME/LIVI`, or `~/.config/LIVI` when unset/empty |
| macOS | `~/Library/Application Support/LIVI` |

Files are written atomically through a temporary file and rename. They are snapshots for readers, not watched input
files: editing one is not a live command. Read them again after a rename; watching only the original file inode can
miss later replacements. Each has its own schema version and UTC write timestamp:

```json
{
  "version": 1,
  "timestamp": "2026-10-31T12:00:00.000Z",
  "payload": {"title":"Song","artist":"Artist"}
}
```

The example payload above is abbreviated. Read the `payload` member rather than treating the envelope itself as
media, GPS, or climate data. Version numbers below are file-schema versions, separate from IPC protocol 11.

| File | Version | Payload | Minimum spacing between writes |
| --- | --- | --- | --- |
| `status.json` | 2 | UI path, projection, audio/phone/assistant activity, navigation announcements, media source. | 50 ms |
| `climate.json` | 1 | The climate block, including source. | 50 ms |
| `gps.json` | 3 | `fix` and `receiver`; receiver omits the satellite list. | 1 s |
| `media.json` | 1 | The `nowPlaying` object. | 50 ms |
| `navigation.json` | 1 | The navigation object. | 50 ms |
| `vehicle.json` | 1 | Selected retained vehicle readings. | 1 s |
| `sdr.json` | 1 | Saved DAB station list and last station. | Saved by DAB persistence rather than the timed state writers. |

Timed writers only write when the payload changes; the intervals are throttles, not a heartbeat. Files may be absent
until data is available and remain on disk after LIVI exits. A timestamp describes a file write, not necessarily a
fresh sensor measurement.

Useful `status.json` payload members include `ui.path`, `projection.active` (`cp`, `aa`, or null),
`projection.streaming`, `projection.phoneType`, `audio.media.playing`, `audio.speech.playing`, `audio.system.playing`,
`phone.active`, `voiceAssistant.active`, `nav.announcing`, and `media.source` (`phone` or `dabplus`). These projection
labels differ from the `carplay`/`androidauto` values in IPC state.

For example, on Linux:

```bash
jq '.payload' "${XDG_CONFIG_HOME:-$HOME/.config}/LIVI/state/media.json"
jq '.payload.ui.path' "${XDG_CONFIG_HOME:-$HOME/.config}/LIVI/state/status.json"
```

### What survives a restart

LIVI restores climate, a valid last GPS position, UI path/media-source preference, DAB stations, and selected vehicle
readings. `vehicle.json` retains numeric values for `fuelPct`, `rangeKm`, `rangeFuelKm`, `rangeElectricKm`,
`consumptionAvgLPer100Km`, `odometerKm`, `odometerTripKm`, `batteryLevelKwh`, `batteryCapacityKwh`, and `tirePressure`.
Restored GPS includes position and optional altitude/heading/fix time, not a fresh speed reading. Persisted settings
can also seed telemetry such as volume and appearance. Dynamic fields such as RPM and reverse are not a durable
telemetry log. Publish current readings when your producer reconnects.

## USB serial vehicle bridge

LIVI automatically searches for a USB serial device whose USB product name is `LIVI Bridge` (case-insensitive).
On Linux it searches `ttyACM*`; on macOS it finds the corresponding callout device through the USB registry. It opens
the device read/write in raw mode at **115200 baud** and retries discovery about every three seconds while disconnected.
There is currently no arbitrary serial-port-path setting for this bridge.

Messages are text lines terminated by `\n`. Keep inbound lines within 512 bytes; the reader limits its unfinished-line
buffer and this is not a bulk-transfer transport. CRLF is tolerated because incoming lines are trimmed.

### Bridge → LIVI

```text
button next
button previous
button scan
car speedKph 42
car rpm 1500
car gear D
car reverse false
car gps.lat 53.5912
car gps.lng 10.015
car climate.driverTargetC 22
```

`button next`, `button previous`, and `button scan` invoke media next, previous, and play/pause respectively.
`car <key> <value>` constructs a telemetry patch. Keys can have one optional dot for a nested block; each part starts
with an ASCII letter and continues with letters, digits, or underscores. Use the telemetry field names above:
`car speed 42` would store a custom `speed` key, not update `speedKph`.

The value parser recognizes `true`, `false`, and finite numbers; other text is an unquoted string. It does not parse
JSON objects, arrays, or `null`. Unknown commands are ignored, and there are no per-command acknowledgements.

### LIVI → bridge

LIVI sends changed fields and replays its cached fields when the bridge reconnects:

| Line prefix | Value |
| --- | --- |
| `name` | `LIVI` |
| `brightness` | Rounded display brightness, 0–100 |
| `media title`, `media artist`, `media album`, `media app` | Text |
| `media duration`, `media position` | Milliseconds |
| `media playing` | `true` or `false` |
| `nav idle-text` | Localized no-route text |
| `nav active` | `true` or `false` |
| `nav maneuver` | Numeric maneuver code |
| `nav maneuver-text`, `nav distance`, `nav road`, `nav destination` | Display text |
| `nav eta` | Local `HH:MM` when numeric ETA is available, otherwise supplied ETA text |
| `nav destination-distance` | Metres |
| `nav time-left` | Seconds |

Text can contain spaces: parse the known multiword field prefix, not just the first token. Newlines in text are
replaced with spaces. Some values are emitted only when known. When navigation ends, `nav active false` tells the
bridge to stop displaying old guidance; old detail fields are not all blanked. Media text is blanked after about
four seconds when the now-playing object becomes empty. Keep that retained-field behavior in mind in a bridge UI.

## Integration checks and troubleshooting

| Symptom | Check |
| --- | --- |
| Port 4000 refuses connections | Ensure the core is running, inspect `[telemetry]` logs for bind failures, and check the host/firewall. |
| HTTP succeeds but nothing changes on screen | Check exact field spelling/types and consumer support. Stored custom keys and unsupported phone features still receive `204`. |
| A WebSocket connection cannot exchange JSON | Use a Socket.IO client on namespace `/` and path `/socket.io/`. |
| Phone media/navigation are missing from Socket.IO updates | Read local IPC `nowPlaying`/`navigation` or the corresponding state files. |
| Old values remain after a sensor stops | Omitted fields retain their previous values; send explicit transitions and handle freshness in your integration. |
| Climate updates loop | Compare actual changed fields and use `climate.source`; avoid echoing every full snapshot. |
| Local IPC immediately closes | Check peer UID, hello-first framing, protocol version, and the core's logs. |
| Local socket is not found | Use the logged path; check `XDG_RUNTIME_DIR`/`TMPDIR` and the user running LIVI. |
| A repeated `path` does not reopen a page | Non-projection page commands trigger on a stored-path change; `/projection` is the repeatable special case. |
| State file appears frozen | Writes happen only on changes, paths are atomically replaced, and files can outlive the process. |

For implementation details, start with [telemetry.rs](native/crates/livi-core/src/telemetry.rs),
[the IPC server](native/crates/livi-core/src/server.rs), [message definitions](native/crates/livi-core-proto/src/message.rs),
[state file writers](native/crates/livi-core/src/state_files.rs), and
[the serial bridge](native/crates/livi-core/src/car_bridge.rs). See [DEBUGGING.md](DEBUGGING.md) for runtime logging.
