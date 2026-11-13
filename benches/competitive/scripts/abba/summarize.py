#!/usr/bin/env python3
"""Summarize retained raw paired receipts without pooling different repeats."""
import json
import math
from pathlib import Path
import statistics
import sys

path = Path(sys.argv[1])
data = json.loads(path.read_text())
fair = 'engine' in data['blocks'][0]
labels = ('turbo', 'jspdf') if fair else ('A', 'B')
key = 'engine' if fair else 'label'
indexed = {(b['repeat'], b[key]): b['receipt'] for b in data['blocks']}
repeats = sorted({r for r, _ in indexed})
ids = data['ids'].split(',')
rows = []
for workload in ids:
    for mode in ('warm', 'cold'):
        stats = {}
        medians = {}
        for label in labels:
            summaries = []
            for repeat in repeats:
                result = next(r for r in indexed[(repeat, label)]['results'] if r['id'] == workload)
                summary = result['modes'][mode]['summary']
                if summary['n'] != data['runs'] or not all(math.isfinite(summary[k]) and summary[k] > 0 for k in ('p50', 'p95')):
                    raise ValueError(f'invalid summary {workload} {repeat} {label} {mode}')
                summaries.append(summary)
            stats[label] = {k: statistics.median(s[k] for s in summaries) for k in ('p50', 'p95')}
            medians[label] = [s['p50'] for s in summaries]
        row = {'workload': workload, 'mode': mode, 'median_of_repeat_stats_ms': stats}
        if fair:
            row['turbo_vs_jspdf_p50_ratio'] = stats['turbo']['p50'] / stats['jspdf']['p50']
        else:
            gains = [(a - b) / a * 100 for a, b in zip(medians['A'], medians['B'])]
            row['candidate_p50_reduction_percent_per_pair'] = gains
            row['all_pairs_faster'] = all(g > 0 for g in gains)
            row['median_pair_reduction_percent'] = statistics.median(gains)
        rows.append(row)
out = path.with_name('summary.json')
out.write_text(json.dumps({'source': str(path), 'repeats': repeats, 'rows': rows}, indent=2))
print(out)
