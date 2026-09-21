// hidmaestro-bridge — NDJSON-over-stdio host for the HIDMaestro .NET SDK.
//
// The Rust `hidmaestro` crate spawns this process and sends one JSON object
// per line on stdin:
//     {"id":1,"method":"create_controller","params":{"profile_id":"xbox-360-wired"}}
// It replies per line on stdout:
//     {"id":1,"ok":true,"result":{...}}
//     {"id":1,"ok":false,"error":"..."}
// Raw and decoded output reports (rumble/LED/FFB) arrive unsolicited:
//     {"event":"output","data":{...}}
//
// Run elevated — driver install and device creation need administrator.

using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Nodes;
using HIDMaestro;

using var ctx = new HMContext();
var controllers = new Dictionary<string, HMController>(StringComparer.Ordinal);
var writeLock = new object();
int nextKey = 0;
bool shutdownRequested = false;

void Write(JsonObject obj)
{
    lock (writeLock)
        Console.Out.WriteLine(obj.ToJsonString());
}

JsonObject Ok(ulong id, JsonNode? result) =>
    new() { ["id"] = id, ["ok"] = true, ["result"] = result };
JsonObject Fail(ulong id, string error) =>
    new() { ["id"] = id, ["ok"] = false, ["error"] = error };

static ulong ReqId(JsonObject req) => req["id"]?.GetValue<ulong>() ?? 0;
static string ReqStr(JsonObject p, string name) =>
    p[name]?.GetValue<string>() ?? throw new ArgumentException($"missing '{name}'");

static JsonObject ProfileJson(HMProfile p) => new()
{
    ["id"] = p.Id,
    ["name"] = p.Name,
    ["vendor"] = p.Vendor,
    ["vendor_id"] = p.VendorId,
    ["product_id"] = p.ProductId,
    ["display_name"] = p.DisplayName,
    ["connection"] = p.Connection,
    ["backend"] = p.Backend,
    ["button_count"] = p.ButtonCount,
    ["axis_count"] = p.AxisCount,
    ["has_hat"] = p.HasHat,
    ["deployable"] = p.IsDeployable,
    ["requires_usbip_backend"] = p.RequiresUsbipBackend,
};

void HookEvents(HMController ctrl, string key)
{
    // Every packet is forwarded; decoded packets also emit the enriched event below.
    ctrl.OutputReceived += (_, packet) =>
    {
        var raw = new byte[packet.Data.Length + 1];
        raw[0] = packet.ReportId;
        packet.Data.Span.CopyTo(raw.AsSpan(1));
        Write(new JsonObject
        {
            ["event"] = "output",
            ["data"] = new JsonObject
            {
                ["controller"] = key,
                ["report_id"] = packet.ReportId,
                ["fields"] = new JsonObject(),
                ["raw"] = new JsonArray(raw.Select(static b => (JsonNode?)JsonValue.Create(b)).ToArray()),
                ["crc_valid"] = false, // Raw packets have not been CRC-checked.
            },
        });
    };

    ctrl.OutputDecoded += (_, e) =>
    {
        var fields = new JsonObject();
        foreach (var kv in e.Fields)
            fields[kv.Key] = kv.Value switch
            {
                byte[] bytes => new JsonArray(bytes.Select(static b => (JsonNode?)JsonValue.Create(b)).ToArray()),
                null => null,
                _ => JsonSerializer.SerializeToNode(kv.Value, kv.Value.GetType()),
            };
        Write(new JsonObject
        {
            ["event"] = "output",
            ["data"] = new JsonObject
            {
                ["controller"] = key,
                ["report_id"] = e.ReportId,
                ["fields"] = fields,
                ["raw"] = new JsonArray(e.RawBytes.ToArray().Select(static b => (JsonNode?)JsonValue.Create(b)).ToArray()),
                ["crc_valid"] = e.CrcValid,
            },
        });
    };
}

HMController RequireController(string key) =>
    controllers.TryGetValue(key, out var c)
        ? c
        : throw new ArgumentException($"no such controller: {key}");

static HMAxis ParseAxis(JsonNode n)
{
    if (n is JsonValue v)
    {
        if (v.TryGetValue<int>(out var i)) return (HMAxis)i;
        var s = v.GetValue<string>();
        if (Enum.TryParse<HMAxis>(s, true, out var a)) return a;
        if (s.StartsWith("0x")) return (HMAxis)Convert.ToUInt16(s, 16);
        return (HMAxis)ushort.Parse(s);
    }
    throw new ArgumentException("invalid axis key");
}

HMGamepadState BuildState(HMProfile profile, JsonObject s)
{
    var state = new HMGamepadState();

    var std = s["standard_axes"] as JsonObject;
    var axes = new Dictionary<HMAxis, float>();
    if (std is not null)
    {
        float Get(string n, float d) => std[n]?.GetValue<float>() ?? d;
        foreach (var kv in HMGamepadStateHelpers.StandardAxes(
                     profile,
                     Get("left_stick_x", 0.5f), Get("left_stick_y", 0.5f),
                     Get("right_stick_x", 0.5f), Get("right_stick_y", 0.5f),
                     Get("left_trigger", 0f), Get("right_trigger", 0f)))
            axes[kv.Key] = kv.Value;
    }
    if (s["axes"] is JsonObject axisObj)
        foreach (var kv in axisObj)
        {
            var axis = kv.Key.StartsWith("0x")
                ? (HMAxis)Convert.ToUInt16(kv.Key, 16)
                : Enum.TryParse<HMAxis>(kv.Key, true, out var a)
                    ? a
                    : (HMAxis)ushort.Parse(kv.Key);
            axes[axis] = kv.Value!.GetValue<float>();
        }
    if (axes.Count > 0) state.Axes = axes;

    state.Buttons = (HMButton)(s["buttons"]?.GetValue<uint>() ?? 0);
    state.Hat = (HMHat)(s["hat"]?.GetValue<byte>() ?? 0);
    if (s["hat_degrees"] is JsonNode hd && hd is not null)
        state.HatDegrees = hd.GetValue<float>();

    if (s["battery_level"] is JsonNode bl && bl is not null)
        state.BatteryLevel = bl.GetValue<byte>();
    if (s["battery_charging"] is JsonNode bc && bc is not null)
        state.BatteryCharging = bc.GetValue<bool>();
    if (s["accel_g"] is JsonArray ag && ag.Count == 3)
    {
        state.AccelGX = ag[0]!.GetValue<float>();
        state.AccelGY = ag[1]!.GetValue<float>();
        state.AccelGZ = ag[2]!.GetValue<float>();
    }
    if (s["gyro_dps"] is JsonArray gd && gd.Count == 3)
    {
        state.GyroDpsX = gd[0]!.GetValue<float>();
        state.GyroDpsY = gd[1]!.GetValue<float>();
        state.GyroDpsZ = gd[2]!.GetValue<float>();
    }
    return state;
}

JsonNode? Dispatch(string method, JsonObject p)
{
    switch (method)
    {
        case "ping":
            return "pong";

        case "is_driver_installed":
            return ctx.IsDriverInstalled;

        case "install_driver":
            ctx.InstallDriver();
            return null;

        case "install_usbip_backend":
            HMContext.InstallUsbipBackend(msg =>
                Write(new JsonObject { ["event"] = "log", ["data"] = msg }));
            return null;

        case "load_profiles":
            return p["dir"] is JsonNode d
                ? ctx.LoadProfilesFromDirectory(d.GetValue<string>())
                : ctx.LoadDefaultProfiles();

        case "list_profiles":
            return new JsonArray(ctx.AllProfiles.Select(p => (JsonNode)ProfileJson(p)).ToArray());

        case "get_profile":
        {
            var prof = ctx.GetProfile(ReqStr(p, "id"));
            return prof is null ? null : ProfileJson(prof);
        }

        case "create_controller":
        {
            if (ctx.AllProfiles.Count == 0)
                ctx.LoadDefaultProfiles();
            var profileId = ReqStr(p, "profile_id");
            var profile = ctx.GetProfile(profileId)
                ?? throw new ArgumentException($"profile '{profileId}' not found");
            var key = $"c{++nextKey}";
            HMController ctrl = p["index"] is JsonNode idx
                ? ctx.CreateControllerAt(idx.GetValue<int>(), profile, key)
                : ctx.CreateController(profile, key);
            controllers[key] = ctrl;
            HookEvents(ctrl, key);
            return new JsonObject { ["key"] = key, ["profile_id"] = profileId };
        }

        case "remove_controller":
        {
            var key = ReqStr(p, "key");
            RequireController(key).Dispose();
            controllers.Remove(key);
            return null;
        }

        case "list_controllers":
            return new JsonArray(controllers.Select(kv =>
                (JsonNode)new JsonObject
                {
                    ["key"] = kv.Key,
                    ["profile_id"] = kv.Value.Profile.Id,
                }).ToArray());

        case "remove_all_controllers":
            foreach (var controller in controllers.Values)
                controller.Dispose();
            controllers.Clear();
            HMContext.RemoveAllVirtualControllers();
            return null;

        case "submit_state":
        {
            var ctrl = RequireController(ReqStr(p, "key"));
            var so = p["state"] as JsonObject
                ?? throw new ArgumentException("missing 'state'");
            ctrl.SubmitState(BuildState(ctrl.Profile, so));
            return null;
        }

        case "submit_raw_report":
        {
            var ctrl = RequireController(ReqStr(p, "key"));
            ctrl.SubmitRawReport(Convert.FromBase64String(ReqStr(p, "data_b64")));
            return null;
        }

        case "finalize_names":
            ctx.FinalizeNames();
            return null;

        case "shutdown":
            shutdownRequested = true;
            return null;

        default:
            throw new ArgumentException($"unknown method '{method}'");
    }
}

string? line;
while ((line = Console.In.ReadLine()) is not null)
{
    if (string.IsNullOrWhiteSpace(line)) continue;
    JsonObject? req = null;
    ulong id = 0;
    try
    {
        req = JsonNode.Parse(line) as JsonObject
            ?? throw new ArgumentException("request must be a JSON object");
        id = ReqId(req);
        var method = ReqStr(req, "method");
        var p = req["params"] as JsonObject ?? new JsonObject();
        Write(Ok(id, Dispatch(method, p)));
        if (shutdownRequested) break;
    }
    catch (JsonException e)
    {
        Write(Fail(id, $"bad json: {e.Message}"));
    }
    catch (Exception e)
    {
        Write(Fail(id, $"{e.GetType().Name}: {e.Message}"));
    }
}
