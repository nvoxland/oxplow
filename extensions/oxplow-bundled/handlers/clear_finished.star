# oxplow.work.clear_finished { thread_id }: the person cleared the
# thread's Finished list in the Work panel. Recorded as an event whose
# subject names the thread; `finished_cleared` reads the latest per thread,
# and the panel leaves out what finished before it.

def transform(x):
    thread = "thread:thr%s" % x["input"]["thread_id"]
    return {
        "commands": [],
        "events": [{"type": "oxplow_bundled.finished_cleared", "subject": [thread], "payload": {}}],
        "result": {"thread": thread},
    }
