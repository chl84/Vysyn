#!/usr/bin/env python3
"""Release presentation/RSS/idle CPU measurements on the current Linux desktop."""
import json
import os
from pathlib import Path
import re
import statistics
import subprocess
import time

root = Path(__file__).resolve().parent.parent
out = root/'artifacts'
out.mkdir(exist_ok=True)
args = [str(root/'target/release/vysyn'),'--trace','--smoke-ms','800',str(root/'artifacts/bench-images/gradient.png')]
runs = []
for i in range(10):
    start = time.perf_counter()
    r = subprocess.run(args,capture_output=True,text=True,timeout=10,check=True)
    (out/f'startup-{i}.log').write_text(r.stderr)
    def number(key):
        return float(re.search(r'\b'+key+r'=([0-9.]+)',r.stderr)[1])
    runs.append({'window_ms':number('window_ms'),'gpu_ready_ms':number('gpu_ready_ms'),
                 'first_present_ms':number('first_present_ms'),'process_total_ms':(time.perf_counter()-start)*1000})
log_path = out/'idle-process.log'
with log_path.open('w') as log:
    proc = subprocess.Popen([args[0],'--trace','--smoke-ms','5000',args[-1]],stdout=log,stderr=log)
    try:
        deadline = time.monotonic()+3
        while 'directory_files=' not in log_path.read_text() and time.monotonic()<deadline:
            if proc.poll() is not None: raise RuntimeError(log_path.read_text())
            time.sleep(0.02)
        time.sleep(0.5) # allow both adjacent preloads to finish
        status = Path(f'/proc/{proc.pid}/status').read_text()
        rss_kib = int(re.search(r'VmRSS:\s+(\d+)',status)[1])
        peak_kib = int(re.search(r'VmHWM:\s+(\d+)',status)[1])
        def cpu_ticks():
            stat = Path(f'/proc/{proc.pid}/stat').read_text().split(') ',1)[1].split()
            return int(stat[11])+int(stat[12])
        t0,c0 = time.monotonic(),cpu_ticks()
        time.sleep(2)
        cpu_percent = (cpu_ticks()-c0)/os.sysconf('SC_CLK_TCK')/(time.monotonic()-t0)*100
        gpu = {}
        for fd in Path(f'/proc/{proc.pid}/fdinfo').iterdir():
            try: content = fd.read_text()
            except OSError: continue
            for key,value in re.findall(r'(drm-memory-[\w-]+):\s+(\d+) KiB',content):
                gpu[key] = max(gpu.get(key,0),int(value))
        proc.wait(timeout=7)
        assert proc.returncode==0
    finally:
        if proc.poll() is None: proc.terminate();proc.wait(timeout=3)
report = {'runs':runs,'median_first_present_ms':statistics.median(r['first_present_ms'] for r in runs),
          'min_first_present_ms':min(r['first_present_ms'] for r in runs),
          'max_first_present_ms':max(r['first_present_ms'] for r in runs),
          'rss_kib':rss_kib,'peak_rss_kib':peak_kib,'idle_cpu_percent_one_core':cpu_percent,
          'gpu_driver_memory_kib':gpu}
(out/'desktop-performance.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(report,indent=2))
