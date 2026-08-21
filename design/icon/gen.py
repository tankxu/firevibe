#!/usr/bin/env python3
"""生成应用图标 icon.svg。改图标就改这里，别手改 svg。

外形是超椭圆（n≈5），很接近 Apple 从 Big Sur 起用的连续圆角；
遥控器剪影**故意压过比例** —— 实机是 3.5:1，太瘦，图标里用约 2:1，
好把麦克风键 / 方向环 / 四个应用键三层特征都塞进去，一眼认得出是遥控器。
"""
import math, pathlib

C, INSET = 1024, 84            # 画布 / 四周留白
W, H, R = 342.0, 670.0, 96.0   # 剪影尺寸与圆角
ANG = -14                      # 微微左倾，像被握着
MIC_R, MIC_CY = 52, 112
RING_CY, RING_R, RING_SW = 300, 111, 33
OK_R = 46
KEY_W, KEY_H, KEY_R = 104, 44, 15
KEY_X = (42, W - 42 - KEY_W)
KEY_Y = (486, 548)


def squircle(cx, cy, half, n=5.0, steps=240):
    pts = []
    for i in range(steps):
        t = 2 * math.pi * i / steps
        ct, st = math.cos(t), math.sin(t)
        pts.append((cx + half * math.copysign(abs(ct) ** (2 / n), ct),
                    cy + half * math.copysign(abs(st) ** (2 / n), st)))
    return "M " + " L ".join(f"{x:.2f},{y:.2f}" for x, y in pts) + " Z"


tile = squircle(C / 2, C / 2, (C - 2 * INSET) / 2)
keys = "\n      ".join(
    f'<rect x="{x}" y="{y}" width="{KEY_W}" height="{KEY_H}" rx="{KEY_R}" fill="#ccd7e3"/>'
    for y in KEY_Y for x in KEY_X
)

svg = f'''<svg xmlns="http://www.w3.org/2000/svg" width="{C}" height="{C}" viewBox="0 0 {C} {C}">
  <defs>
    <linearGradient id="tile" x1="0" y1="0" x2="0.85" y2="1">
      <stop offset="0%"   stop-color="#2ad4fb"/>
      <stop offset="42%"  stop-color="#0ea5e0"/>
      <stop offset="100%" stop-color="#03669f"/>
    </linearGradient>
    <linearGradient id="gloss" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0%"   stop-color="#ffffff" stop-opacity=".30"/>
      <stop offset="38%"  stop-color="#ffffff" stop-opacity=".06"/>
      <stop offset="100%" stop-color="#ffffff" stop-opacity="0"/>
    </linearGradient>
    <linearGradient id="rim" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0%"   stop-color="#ffffff" stop-opacity=".55"/>
      <stop offset="45%"  stop-color="#ffffff" stop-opacity=".10"/>
      <stop offset="100%" stop-color="#ffffff" stop-opacity=".22"/>
    </linearGradient>
    <linearGradient id="shell" x1="0" y1="0" x2="0.35" y2="1">
      <stop offset="0%"   stop-color="#ffffff"/>
      <stop offset="58%"  stop-color="#f1f5f9"/>
      <stop offset="100%" stop-color="#d8e2ec"/>
    </linearGradient>
    <linearGradient id="mic" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0%"   stop-color="#19b6ea"/>
      <stop offset="100%" stop-color="#0472a3"/>
    </linearGradient>
    <filter id="drop" x="-40%" y="-40%" width="180%" height="180%">
      <feDropShadow dx="0" dy="18" stdDeviation="22" flood-color="#023c59" flood-opacity=".34"/>
    </filter>
    <clipPath id="clip"><path d="{tile}"/></clipPath>
  </defs>

  <path d="{tile}" fill="url(#tile)"/>
  <g clip-path="url(#clip)">
    <path d="{tile}" fill="url(#gloss)"/>
    <g filter="url(#drop)"
       transform="translate({C/2:.1f} {C/2:.1f}) rotate({ANG}) translate({-W/2:.1f} {-H/2:.1f})">
      <rect x="0" y="0" width="{W}" height="{H}" rx="{R}" fill="url(#shell)"/>
      <circle cx="{W/2}" cy="{MIC_CY}" r="{MIC_R}" fill="url(#mic)"/>
      <circle cx="{W/2}" cy="{RING_CY}" r="{RING_R}" fill="none" stroke="#c6d2df" stroke-width="{RING_SW}"/>
      <circle cx="{W/2}" cy="{RING_CY}" r="{OK_R}" fill="#e8eef4"/>
      {keys}
    </g>
  </g>
  <path d="{tile}" fill="none" stroke="url(#rim)" stroke-width="3"/>
</svg>
'''
out = pathlib.Path(__file__).with_name("icon.svg")
out.write_text(svg)
print(f"wrote {out} ({len(svg)} bytes)")
