"""Generates docs/brand/logo.svg (also app/src-tauri/icons/icon.svg): contour rings around a peak.
Run: python3 gen_logo.py  -> writes icon.svg in the current directory."""
import math, sys
def ring(cx, cy, r, harm, n=12, rot=0):
    pts=[]
    for i in range(n):
        t=2*math.pi*i/n+rot
        k=1+sum(a*math.sin(f*t+p) for a,f,p in harm)
        pts.append((cx+r*k*math.cos(t), cy+r*k*math.sin(t)))
    # closed Catmull-Rom -> cubic bezier
    d=f"M{pts[0][0]:.1f} {pts[0][1]:.1f}"
    for i in range(n):
        p0,p1,p2,p3=pts[i-1],pts[i],pts[(i+1)%n],pts[(i+2)%n]
        c1=(p1[0]+(p2[0]-p0[0])/6, p1[1]+(p2[1]-p0[1])/6)
        c2=(p2[0]-(p3[0]-p1[0])/6, p2[1]-(p3[1]-p1[1])/6)
        d+=f" C{c1[0]:.1f} {c1[1]:.1f} {c2[0]:.1f} {c2[1]:.1f} {p2[0]:.1f} {p2[1]:.1f}"
    return d+" Z"
H=[(0.07,2,0.6),(0.05,3,2.1),(0.03,5,4.0)]
def rings(scale=1.0, ox=0, oy=0):
    # peak offset up-right; rings grow toward lower-left
    spec=[(556,456,82,"#e4a548"),(542,472,154,"#b49be8"),(527,490,228,"#7f9ff2"),(512,512,302,"#5cc4d4")]
    out=[]
    for j,(cx,cy,r,c) in enumerate(spec):
        h=[(a*(0.6+0.25*j),f,p+0.35*j) for a,f,p in H]
        out.append((ring(ox+cx*scale,oy+cy*scale,r*scale,h),c))
    return out, (ox+566*scale, oy+448*scale)
def icon(bg="#17202c", sw=40, tile=True):
    rs,(px,py)=rings()
    s='<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024">\n'
    if tile: s+=f'  <rect x="64" y="64" width="896" height="896" rx="200" fill="{bg}"/>\n'
    s+=f'  <g fill="none" stroke-width="{sw}" stroke-linejoin="round">\n'
    for d,c in rs: s+=f'    <path d="{d}" stroke="{c}"/>\n'
    s+=f'  </g>\n  <circle cx="{px:.0f}" cy="{py:.0f}" r="28" fill="#f3f4f7"/>\n</svg>\n'
    return s
open("icon.svg","w").write(icon())
