"""SQLite persistence and validation for the local job pipeline dashboard."""
from __future__ import annotations

import csv
import json
import os
import sqlite3
from contextlib import contextmanager
from datetime import datetime, timezone
from pathlib import Path
from urllib.parse import urlsplit

ROOT = Path(os.environ.get("LORCA_JOBS_ROOT", str(Path.home()/"Library/Application Support/lorca/job-apps")))
DB_PATH = Path(os.getenv("JOB_PIPELINE_DB", str(ROOT / "pipeline.sqlite3")))
STATUSES = ["Discovered", "Researching", "Ready for review", "Approved to apply", "Applying", "Applied", "Interviewing", "Rejected", "Offer", "Archived"]
PRIORITIES = ["High", "Medium", "Low"]
VERIFICATION = ["Unverified", "Verified", "Expired"]
OUTREACH = ["Not started", "Drafted", "Approved", "Sent", "Replied"]
EDITABLE = {"company", "title", "location", "stage", "priority", "verification", "job_url", "source_url", "verified_at", "salary", "tech_stack", "description", "notes", "recruiter_name", "recruiter_contact", "resume_file", "cover_letter_file", "outreach_stage", "email_subject", "email_draft", "application_approved", "outreach_approved"}
FIELDS = ["company", "title", "location", "job_url", "source_url", "salary", "priority", "stage", "verification", "verified_at", "tech_stack", "description", "notes", "recruiter_name", "recruiter_contact", "resume_file", "cover_letter_file", "outreach_stage", "email_subject", "email_draft", "application_approved", "outreach_approved"]


def now():
    return datetime.now(timezone.utc).isoformat(timespec="seconds")


@contextmanager
def db():
    DB_PATH.parent.mkdir(parents=True, exist_ok=True)
    cx = sqlite3.connect(DB_PATH, timeout=20)
    cx.row_factory = sqlite3.Row
    try:
        yield cx
        cx.commit()
    finally:
        cx.close()


def init_db():
    with db() as cx:
        cx.execute("""
        CREATE TABLE IF NOT EXISTS jobs (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          company TEXT NOT NULL,
          title TEXT NOT NULL,
          location TEXT DEFAULT 'New York, NY',
          job_url TEXT DEFAULT '', source_url TEXT DEFAULT '', salary TEXT DEFAULT '',
          priority TEXT DEFAULT 'Medium', stage TEXT DEFAULT 'Discovered',
          verification TEXT DEFAULT 'Unverified', verified_at TEXT DEFAULT '',
          tech_stack TEXT DEFAULT '', description TEXT DEFAULT '', notes TEXT DEFAULT '',
          recruiter_name TEXT DEFAULT '', recruiter_contact TEXT DEFAULT '',
          resume_file TEXT DEFAULT '', cover_letter_file TEXT DEFAULT '',
          outreach_stage TEXT DEFAULT 'Not started', email_subject TEXT DEFAULT '', email_draft TEXT DEFAULT '',
          application_approved INTEGER NOT NULL DEFAULT 0,
          outreach_approved INTEGER NOT NULL DEFAULT 0,
          created_at TEXT NOT NULL, updated_at TEXT NOT NULL
        )""")
        cx.execute("CREATE INDEX IF NOT EXISTS jobs_stage_idx ON jobs(stage)")
        cx.execute("CREATE INDEX IF NOT EXISTS jobs_company_idx ON jobs(company)")
        cx.execute("""CREATE TABLE IF NOT EXISTS history (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          job_id INTEGER NOT NULL,
          action TEXT NOT NULL,
          happened_at TEXT NOT NULL,
          FOREIGN KEY(job_id) REFERENCES jobs(id)
        )""")


def validate(data, partial=False):
    if not isinstance(data, dict):
        raise ValueError("Expected a JSON object")
    result = {}
    for key, val in data.items():
        if key not in EDITABLE:
            continue
        if key in {"application_approved", "outreach_approved"}:
            if not isinstance(val, bool):
                raise ValueError(f"{key} must be a boolean")
            result[key] = int(val)
        else:
            if not isinstance(val, str):
                raise ValueError(f"{key} must be text")
            limit = 20000 if key in {"description", "notes", "email_draft"} else 1024
            if len(val) > limit:
                raise ValueError(f"{key} exceeds {limit} characters")
            result[key] = val.strip()
    for key, allowed in (("stage", STATUSES), ("priority", PRIORITIES), ("verification", VERIFICATION), ("outreach_stage", OUTREACH)):
        if key in result and result[key] not in allowed:
            raise ValueError(f"Invalid {key}")
    for key in ("job_url", "source_url"):
        if result.get(key):
            url = urlsplit(result[key])
            if url.scheme not in ("https", "http") or not url.netloc:
                raise ValueError(f"{key} must be a valid http(s) URL")
    if not partial or "company" in result or "title" in result:
        if not partial and (not result.get("company") or not result.get("title")):
            raise ValueError("Company and job title are required")
        if "company" in result and not result["company"]:
            raise ValueError("Company cannot be empty")
        if "title" in result and not result["title"]:
            raise ValueError("Title cannot be empty")
    return result


def list_jobs():
    with db() as cx:
        return [dict(r) for r in cx.execute("SELECT * FROM jobs ORDER BY updated_at DESC, id DESC")]


def get_job(job_id):
    with db() as cx:
        row = cx.execute("SELECT * FROM jobs WHERE id=?", (job_id,)).fetchone()
        return dict(row) if row else None


def create_job(data):
    data = validate(data)
    data.setdefault("location", "New York, NY")
    stamp = now()
    data.update(created_at=stamp, updated_at=stamp)
    cols = list(data)
    with db() as cx:
        cur = cx.execute(f"INSERT INTO jobs ({','.join(cols)}) VALUES ({','.join(['?']*len(cols))})", [data[k] for k in cols])
        job_id = cur.lastrowid
        cx.execute("INSERT INTO history(job_id,action,happened_at) VALUES(?,?,?)", (job_id, "Job added", stamp))
    return get_job(job_id)


def update_job(job_id, data):
    data = validate(data, partial=True)
    if not data:
        raise ValueError("No editable fields supplied")
    old = get_job(job_id)
    if not old:
        return None
    # Updating a posting or its source invalidates previously granted approvals.
    if any(k in data and data[k] != old[k] for k in ("job_url", "company", "title")):
        data["application_approved"] = 0
        data["outreach_approved"] = 0
    if "email_draft" in data and data["email_draft"] != old["email_draft"]:
        data["outreach_approved"] = 0
    data["updated_at"] = now()
    keys = list(data)
    with db() as cx:
        cx.execute(f"UPDATE jobs SET {','.join(k+'=?' for k in keys)} WHERE id=?", [data[k] for k in keys] + [job_id])
        changes = ", ".join(k for k in keys if k != "updated_at")
        cx.execute("INSERT INTO history(job_id,action,happened_at) VALUES(?,?,?)", (job_id, f"Updated: {changes}", data["updated_at"]))
    return get_job(job_id)


def get_history(job_id):
    with db() as cx:
        return [dict(r) for r in cx.execute("SELECT action,happened_at FROM history WHERE job_id=? ORDER BY id DESC LIMIT 50", (job_id,))]


def export_csv(output_path):
    entries = list_jobs()
    with open(output_path, "w", newline="", encoding="utf-8") as f:
        writer = csv.DictWriter(f, fieldnames=["id"] + FIELDS + ["created_at", "updated_at"], extrasaction="ignore")
        writer.writeheader()
        for row in entries:
            # Escape formula-leading cells in CSV to avoid spreadsheet injection.
            sanitized = {k: ("'" + v if isinstance(v, str) and v.lstrip().startswith(("=", "+", "-", "@")) else v) for k, v in row.items()}
            writer.writerow(sanitized)
    return len(entries)


def import_json(input_path):
    payload = json.loads(Path(input_path).read_text(encoding="utf-8"))
    if isinstance(payload, dict) and "jobs" in payload:
        payload = payload["jobs"]
    if isinstance(payload, dict):
        payload = [payload]
    if not isinstance(payload, list):
        raise ValueError("Expected one job object, an array of jobs, or {jobs: [...]}")
    count = 0
    for entry in payload:
        create_job(entry)
        count += 1
    return count
