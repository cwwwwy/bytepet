/**
 * The IPC client must never reject: every command resolves to a discriminated
 * `{ ok, error }` result so components can render failures without try/catch.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invoke(...args),
  convertFileSrc: (path: string) => `asset://localhost/${path}`,
}));

const { describeError, invokeCommand, ipc, isOk, unwrapOr } = await import("./ipc");

const settings = { schemaVersion: 1, activePet: null } as never;

describe("invokeCommand", () => {
  beforeEach(() => {
    invoke.mockReset();
  });

  it("wraps a successful command in { ok: true, value }", async () => {
    invoke.mockResolvedValueOnce(settings);
    const result = await invokeCommand<typeof settings>("get_settings");
    expect(result).toEqual({ ok: true, value: settings });
    expect(isOk(result)).toBe(true);
  });

  it("passes the command name and argument object through", async () => {
    invoke.mockResolvedValueOnce(null);
    await ipc.validatePet("/tmp/pet");
    expect(invoke).toHaveBeenCalledWith("validate_pet", { path: "/tmp/pet" });
  });

  it("sends no arguments for no-arg commands", async () => {
    invoke.mockResolvedValueOnce([]);
    await ipc.listConversations();
    expect(invoke).toHaveBeenCalledWith("list_conversations", undefined);
  });

  it("sends optional arguments when provided", async () => {
    invoke.mockResolvedValueOnce([]);
    await ipc.listConversations("persona-1");
    expect(invoke).toHaveBeenCalledWith("list_conversations", { personaId: "persona-1" });
  });

  it("wraps the settings payload", async () => {
    invoke.mockResolvedValueOnce(settings);
    await ipc.saveSettings(settings);
    expect(invoke).toHaveBeenCalledWith("save_settings", { config: settings });
  });

  it("surfaces string rejections as { ok: false, error }", async () => {
    invoke.mockRejectedValueOnce("provider not found");
    const result = await invokeCommand("test_provider", { id: "x" });
    expect(result).toEqual({ ok: false, error: "provider not found" });
    expect(isOk(result)).toBe(false);
  });

  it("surfaces Error rejections with their message", async () => {
    invoke.mockRejectedValueOnce(new Error("boom"));
    const result = await ipc.testProvider("x");
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.error).toBe("boom");
  });

  it("surfaces object rejections with a message field", async () => {
    invoke.mockRejectedValueOnce({ message: "denied" });
    const result = await ipc.getBootstrapState();
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.error).toBe("denied");
  });

  it("never throws, even for exotic rejections", async () => {
    invoke.mockRejectedValueOnce(42);
    const result = await invokeCommand("whatever");
    expect(result).toEqual({ ok: false, error: "42" });
  });

  it("passes the conversation id for cancel_message", async () => {
    invoke.mockResolvedValueOnce(undefined);
    await ipc.cancelMessage("conv-1");
    expect(invoke).toHaveBeenCalledWith("cancel_message", { conversationId: "conv-1" });
  });

  it("passes the provider id for has_api_key", async () => {
    invoke.mockResolvedValueOnce(true);
    await ipc.hasApiKey("provider-1");
    expect(invoke).toHaveBeenCalledWith("has_api_key", { providerId: "provider-1" });
  });
});

describe("describeError", () => {
  it("normalises every rejection shape", () => {
    expect(describeError("plain")).toBe("plain");
    expect(describeError(new Error("nope"))).toBe("nope");
    expect(describeError({ error: "nested" })).toBe("nested");
    expect(describeError({ message: "msg" })).toBe("msg");
    expect(describeError(undefined)).toBe("undefined");
    expect(describeError({ a: 1 })).toBe('{"a":1}');
  });
});

describe("unwrapOr", () => {
  it("returns the value or the fallback", () => {
    expect(unwrapOr({ ok: true, value: 7 }, 0)).toBe(7);
    expect(unwrapOr({ ok: false, error: "x" }, 0)).toBe(0);
  });
});
