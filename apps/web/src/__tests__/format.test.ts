import { describe, expect, it } from "vitest";

import { discoverQueryString, emptyFilters } from "../api";
import {
  daysUntil,
  deadlineLabel,
  formatDate,
  freshnessLabel,
  isAdvertisedVacancy,
  isStale,
  kindLabel,
  locationLabel,
  orUnknown,
  outcomeLabel,
  relativeAge,
  stateLabel,
  supportLabel,
} from "../format";

describe("formatting", () => {
  it("shows an absent value as unknown rather than inventing a default", () => {
    expect(orUnknown(null)).toBe("unknown");
    expect(orUnknown("   ")).toBe("unknown");
    expect(orUnknown("Munich")).toBe("Munich");
    expect(formatDate(null)).toBe("unknown");
    expect(formatDate("not a date")).toBe("unknown");
    expect(formatDate("2026-10-02T11:00:00Z")).toBe("2026-10-02");
  });

  it("keeps the source's own remote wording instead of interpreting it", () => {
    expect(locationLabel({ city: "Seattle", country_code: "US", remote_raw: "Remote - US only" })).toBe(
      "Seattle, US (source says: Remote - US only)",
    );
    expect(locationLabel({ city: null, country_code: null, remote_raw: null })).toBe("location unknown");
  });

  it("labels a never-attempted source distinctly from a failed one", () => {
    expect(outcomeLabel(null)).toBe("Never attempted");
    expect(outcomeLabel("ok_verified_empty")).toBe("Complete sweep, no openings");
    expect(outcomeLabel("http_error")).toBe("HTTP error");
    expect(supportLabel("not_configured")).toBe("Never checked");
    expect(supportLabel("verified_live")).toBe("Verified live");
  });

  it("translates opportunity kinds including the German thesis case", () => {
    expect(kindLabel("master_thesis")).toBe("Master thesis");
    expect(kindLabel("working_student")).toBe("Working student");
    expect(kindLabel("something_new")).toBe("something_new");
  });

  it("reports freshness relative to now and treats an unknown timestamp as stale", () => {
    const now = new Date("2026-10-22T12:00:00Z");
    expect(relativeAge("2026-10-22T11:30:00Z", now)).toBe("under an hour ago");
    expect(relativeAge("2026-10-22T06:00:00Z", now)).toBe("6h ago");
    expect(relativeAge("2026-10-21T12:00:00Z", now)).toBe("1 day ago");
    expect(relativeAge(null, now)).toBe("unknown");
    expect(isStale(null, now)).toBe(true);
    expect(isStale("2026-10-22T11:00:00Z", now)).toBe(false);
    expect(isStale("2026-10-18T11:00:00Z", now)).toBe(true);
  });
});

describe("academic honesty labels", () => {
  it("never describes a thesis topic or a supervisor contact as a vacancy", () => {
    expect(isAdvertisedVacancy("advertised")).toBe(true);
    expect(isAdvertisedVacancy("open_topic")).toBe(false);
    expect(isAdvertisedVacancy("potential_supervisor")).toBe(false);
    expect(isAdvertisedVacancy("unknown")).toBe(false);
    expect(stateLabel("open_topic")).toBe("Open topic, not a funded post");
    expect(stateLabel("potential_supervisor")).toBe("Possible supervisor, nothing advertised");
  });

  it("separates freshness from what the adapter has proven", () => {
    expect(freshnessLabel("fresh")).toBe("Up to date");
    expect(freshnessLabel("stale")).toBe("Stale, overdue for a refresh");
    expect(freshnessLabel("never")).toBe("Never read");
  });

  it("reports a deadline relative to today and keeps an unstated one absent", () => {
    const now = new Date("2026-10-22T12:00:00Z");
    expect(daysUntil(null, now)).toBeNull();
    expect(deadlineLabel(null, now)).toBeNull();
    expect(deadlineLabel("2026-10-22T23:59:59Z", now)).toBe("Deadline today, 2026-10-22");
    expect(deadlineLabel("2026-10-23T23:59:59Z", now)).toBe("Deadline tomorrow, 2026-10-23");
    // 9 remaining days in October plus 30 in November plus 31 in December.
    expect(deadlineLabel("2026-12-31T23:59:59Z", now)).toBe("Deadline in 70 days, 2026-12-31");
    expect(deadlineLabel("2026-10-01T23:59:59Z", now)).toBe("Deadline passed 2026-10-01");
  });
});

describe("discover query string", () => {
  it("omits empty filters so a blank form does not narrow the search", () => {
    expect(discoverQueryString(emptyFilters)).toBe("sort=newest&page=1&per_page=25");
  });

  it("passes every set filter through as a bound query parameter", () => {
    const params = new URLSearchParams(
      discoverQueryString({ ...emptyFilters, q: " firmware ", country: "DE", city: "Munich", kind: "master_thesis", includeClosed: true }),
    );
    expect(params.get("q")).toBe("firmware");
    expect(params.get("country")).toBe("DE");
    expect(params.get("city")).toBe("Munich");
    expect(params.get("type")).toBe("master_thesis");
    expect(params.get("include_closed")).toBe("true");
  });
});
