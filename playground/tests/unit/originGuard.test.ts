import type { IncomingMessage } from "node:http";
import { describe, expect, it } from "vitest";
import {
  ApiError,
  assertJsonContentType,
  assertSameOrigin,
  isAllowedHost,
  isAllowedOrigin
} from "../../server/http";

function fakeReq(options: {
  method?: string;
  headers?: Record<string, string | undefined>;
}): IncomingMessage {
  return {
    method: options.method ?? "GET",
    headers: options.headers ?? {}
  } as unknown as IncomingMessage;
}

describe("isAllowedHost", () => {
  it("accepts loopback hostnames with or without a port", () => {
    expect(isAllowedHost("127.0.0.1")).toBe(true);
    expect(isAllowedHost("127.0.0.1:5174")).toBe(true);
    expect(isAllowedHost("localhost")).toBe(true);
    expect(isAllowedHost("localhost:3112")).toBe(true);
    expect(isAllowedHost("[::1]:5174")).toBe(true);
  });

  it("rejects missing, public, and rebinding hosts", () => {
    expect(isAllowedHost(undefined)).toBe(false);
    expect(isAllowedHost("")).toBe(false);
    expect(isAllowedHost("evil.example.com")).toBe(false);
    expect(isAllowedHost("evil.example.com:5174")).toBe(false);
    // DNS-rebinding name that happens to resolve to loopback is still rejected
    // because the Host header carries the attacker's name, not the IP.
    expect(isAllowedHost("attacker.test")).toBe(false);
  });
});

describe("isAllowedOrigin", () => {
  it("accepts loopback origins and referers", () => {
    expect(isAllowedOrigin("http://127.0.0.1:5174")).toBe(true);
    expect(isAllowedOrigin("http://localhost:5174")).toBe(true);
    expect(isAllowedOrigin("https://localhost")).toBe(true);
    // Referer carries a path; URL parsing still extracts the loopback host.
    expect(isAllowedOrigin("http://127.0.0.1:5174/playground")).toBe(true);
  });

  it("rejects foreign, empty, opaque, and malformed origins", () => {
    expect(isAllowedOrigin(undefined)).toBe(false);
    expect(isAllowedOrigin("")).toBe(false);
    expect(isAllowedOrigin("null")).toBe(false);
    expect(isAllowedOrigin("https://evil.example.com")).toBe(false);
    expect(isAllowedOrigin("not a url")).toBe(false);
  });
});

describe("assertJsonContentType", () => {
  it("passes bodyless methods regardless of content-type", () => {
    expect(() => assertJsonContentType(fakeReq({ method: "GET" }))).not.toThrow();
    expect(() =>
      assertJsonContentType(fakeReq({ method: "GET", headers: { "content-type": "text/plain" } }))
    ).not.toThrow();
  });

  it("accepts application/json (with charset) for bodied methods", () => {
    expect(() =>
      assertJsonContentType(
        fakeReq({ method: "POST", headers: { "content-type": "application/json" } })
      )
    ).not.toThrow();
    expect(() =>
      assertJsonContentType(
        fakeReq({ method: "POST", headers: { "content-type": "application/json; charset=utf-8" } })
      )
    ).not.toThrow();
  });

  it("rejects text/plain and missing content-type on bodied methods with 415", () => {
    for (const headers of [{ "content-type": "text/plain" }, {}, { "content-type": "" }]) {
      try {
        assertJsonContentType(fakeReq({ method: "POST", headers }));
        throw new Error("expected assertJsonContentType to throw");
      } catch (error) {
        expect(error).toBeInstanceOf(ApiError);
        expect((error as ApiError).status).toBe(415);
      }
    }
  });
});

describe("assertSameOrigin", () => {
  it("passes same-origin loopback requests with no cross-origin Origin", () => {
    expect(() =>
      assertSameOrigin(fakeReq({ method: "GET", headers: { host: "127.0.0.1:5174" } }))
    ).not.toThrow();
  });

  it("passes loopback Origin on a loopback Host", () => {
    expect(() =>
      assertSameOrigin(
        fakeReq({
          method: "POST",
          headers: { host: "127.0.0.1:5174", origin: "http://127.0.0.1:5174" }
        })
      )
    ).not.toThrow();
  });

  it("rejects a foreign Origin with 403", () => {
    try {
      assertSameOrigin(
        fakeReq({
          method: "POST",
          headers: { host: "127.0.0.1:5174", origin: "https://evil.example.com" }
        })
      );
      throw new Error("expected assertSameOrigin to throw");
    } catch (error) {
      expect(error).toBeInstanceOf(ApiError);
      expect((error as ApiError).status).toBe(403);
    }
  });

  it("rejects a foreign Referer when no Origin is present", () => {
    try {
      assertSameOrigin(
        fakeReq({
          method: "GET",
          headers: { host: "127.0.0.1:5174", referer: "https://evil.example.com/x" }
        })
      );
      throw new Error("expected assertSameOrigin to throw");
    } catch (error) {
      expect((error as ApiError).status).toBe(403);
    }
  });

  it("rejects a non-loopback Host (DNS-rebinding) with 403", () => {
    try {
      assertSameOrigin(fakeReq({ method: "GET", headers: { host: "evil.example.com" } }));
      throw new Error("expected assertSameOrigin to throw");
    } catch (error) {
      expect((error as ApiError).status).toBe(403);
    }
  });
});
