from pathlib import Path
from xml.sax.saxutils import escape
import cairosvg
root=Path(__file__).parents[1]
out=root/'ui-v13-responsive';out.mkdir(exist_ok=True)
W,H=1600,900
ink='#29322C';sub='#7B827D';accent='#5C765F';bg='#F5F6F3';border='#E4E7E2';surface='#FFFFFF';hover_bg='#EAF0E8'
files=[('□','要件定义.md','2026-11-18 17:36','Markdown','124 KB','已归档'),('□','基本设计.md','2026-11-18 16:12','Markdown','89 KB','已归档'),('□','详细设计.md','2026-11-18 15:52','Markdown','198 KB','已同步'),('□','UT测试计划.xlsx','2026-11-18 15:13','Excel','286 KB','已同步'),('□','architecture.nema','2026-11-18 14:38','NEMA','18 KB','已归档'),('□','.LADONignore','2026-11-18 14:05','规则文件','1 KB','仅本地')]
def svg(mode,sidebar,chat,hover,cloud=False,statusfade=False,emerge=False):
    xsidebar={'wide':240,'icons':66,'none':0}[sidebar];content=xsidebar+28
    p=[]
    def add(s):p.append(s)
    def rect(x,y,w,h,fill=surface,rx=0,stroke='none',extra=''):
        add(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{rx}" fill="{fill}" stroke="{stroke}" {extra}/>')
    def line(x1,y1,x2,y2,c=border):add(f'<path d="M{x1} {y1} L{x2} {y2}" stroke="{c}"/>')
    def t(x,y,s,size=14,color=ink,weight='400',attr=''):
        add(f'<text x="{x}" y="{y}" font-family="Noto Sans CJK SC,Microsoft YaHei,Segoe UI,sans-serif" font-size="{size}" fill="{color}" font-weight="{weight}" {attr}>{escape(str(s))}</text>')
    add(f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}">')
    rect(0,0,W,H,bg)
    rect(17,12,1566,875,'#FFFFFF',16,border)
    t(45,47,'◆',22,accent);t(76,47,'LADON Explorer',20,ink,'600');t(1284,46,'◌  个人空间 / LADON Team   ▾',13,sub)
    t(1482,46,'—    □    ×',18,sub)
    # tabs
    rect(32,64,201,40,'#F0F4EF',10);t(53,90,'□  文件浏览',14,ink,'600');rect(242,64,161,40,'#F7F7F5',10);t(265,90,'□  文档任务',14,sub);t(418,90,'＋',18,sub)
    line(18,109,1583,109)
    # navigation row
    t(47,148,'←    →    ↑',20,sub);rect(213,119,800,40,'#F9FAF8',10,border)
    t(236,145,'此电脑    ›    项目    ›    LADON    ›    设计文档',14,ink)
    rect(1111,119,445,40,'#F9FAF8',11,border);t(1136,145,'⌕   搜索文件、标签与内容',14,sub)
    line(18,173,1583,173)
    # sidebar content
    rect(18,174,xsidebar,645,'#F7F8F6' if xsidebar else '#FFFFFF')
    if xsidebar:
        names=['☆','□','⌕','□','□','◇','◎','○']
        rows=['快速访问','文件浏览','搜索','文档任务','FileSpace','云端存储','历史','LADON Ignore']
        for i,(icon,name) in enumerate(zip(names,rows)):
            y=217+52*i
            if i==1:rect(25,y-26,xsidebar-15,43,hover_bg,10)
            t(39,y,icon,20,accent if i==1 else sub)
            if xsidebar>100:t(80,y,name,14,ink if i==1 else sub,'600' if i==1 else '400')
        t(38,797,'‹' if sidebar=='wide' else '›',20,accent)
    line(18+xsidebar,174,18+xsidebar,818)
    if hover and sidebar=='icons':
        rect(79,297,218,174,'#FFFFFF',14,'#D7DED6')
        t(99,328,'文档任务',15,ink,'600');t(100,366,'□   要件条款',14,sub)
        t(100,401,'□   影响分析',14,sub);t(100,438,'◷   待审批任务',14,sub)
    # toolbar
    t(content,210,'＋  新建     ▢  复制      ↑  分享      □  文档任务  ▾      ✦  AI标签  ▾      ◷  历史      ···',14,ink)
    t(1355,210,'☁  已同步  ▾',14,accent)
    line(content,232,1560,232)
    t(content+10,262,'名称',13,sub);t(content+607,262,'修改时间',13,sub)
    t(content+895,262,'类型',13,sub);t(content+1088,262,'大小',13,sub);t(content+1200,262,'状态',13,sub)
    line(content,274,1556,274)
    for i,(glyph,name,date,ftype,sz,status) in enumerate(files):
        y=301+i*53
        if i==1:rect(content,y-23,1553-content,44,hover_bg,9)
        t(content+16,y,glyph,20,accent);t(content+57,y,name,14,ink,'600' if i==1 else '400')
        if i==1:
            rect(content+245,y-19,50,22,'#F1F4EF',11);t(content+258,y-4,'任务',11,accent)
        t(content+607,y,date,13,sub);t(content+895,y,ftype,13,sub)
        t(content+1088,y,sz,13,sub);t(content+1200,y,'● '+status,12,accent if status!='仅本地' else sub)
        if i==1 and (hover or mode=='hover'):
            t(1505,y,'◷  ···',15,accent)
        line(content,y+15,1556,y+15,'#EDF0EB')
    # emergent panel
    if emerge:
        rect(900,535,595,167,'#FFFFFF',14,'#CCD8CB')
        t(926,571,'✦   发现一个需要审阅的变更',16,accent,'600')
        t(926,603,'REQ-027 最近修改了要件，相关 ST-032 测试尚未更新。',13,ink)
        t(926,630,'证据：要件指纹变化 + 测试关系图 + 测试失败事件',12,sub)
        rect(1323,648,146,35,'#EAF0E8',8);t(1350,672,'查看建议  ›',13,accent,'600')
    # bottom dock
    docky=731 if chat=='open' else 822
    if chat=='open':
        rect(content-4,719,1554-content+8,163,'#FBFCFA',13,'#DDE5DC')
        t(content+18,749,'✦  AI 助手      □  团队通信',14,accent,'600')
        line(content+17,758,1541,758)
        t(content+25,787,'AI · 16:43',12,sub)
        t(content+25,809,'相关设计任务已经关联到 REQ-027，待你审批。',13,ink)
        docky=829
    # status hints fade within dock, bottom anchored
    rect(content-4,822,1554-content+8,60,'#FAFBF9',11,'#DDE5DC')
    if not statusfade:
        t(content+14,858,'◌  6 项',12,sub)
        t(content+89,858,'☁ 已同步',12,accent)
    else:
        t(content+19,858,' ',12,sub)
    rect(content+192,835,135,33,'#F0F3EF',9);t(content+208,857,'✦  AI  ▾',13,accent)
    rect(content+337,835,120,33,'#F0F3EF',9);t(content+351,857,'◇ 团队 ▾',13,accent)
    t(content+483,857,'向 AI 或成员发送消息…',14,sub)
    t(1473,857,'＠  ↑',16,sub)
    rect(1520,831,35,39,'#789273',9);t(1530,857,'➤',16,surface)
    # label footer note outside picture no text; pinned security if requested
    add('</svg>')
    return ''.join(p)
states=[
 ('15_侧栏展开_默认底部Dock','main','wide','closed',False,False,False),
 ('16_侧栏折叠_文件区自适应','main','icons','closed',False,False,False),
 ('17_悬停浮层_不挤压内容','hover','icons','closed',True,False,False),
 ('18_底部AI团队聊天展开','main','wide','open',False,False,False),
 ('19_低优先状态自动淡出','main','icons','closed',False,True,False),
 ('20_多信号涌现任务建议','main','wide','closed',False,False,True)
]
for name,mode,side,chat,hov,fade,emerge in states:
    markup=svg(mode,side,chat,hov,False,fade,emerge)
    (out/(name+'.svg')).write_text(markup,encoding='utf-8')
    cairosvg.svg2png(bytestring=markup.encode('utf-8'),write_to=str(out/(name+'.png')),output_width=1440)
# contact sheet
from PIL import Image,ImageDraw,ImageFont
sheet=Image.new('RGB',(1440,3*465),'#F5F6F3')
for i,(name,*_) in enumerate(states):
    im=Image.open(out/(name+'.png')).convert('RGB').resize((700,395))
    x=(i%2)*720+10;y=(i//2)*465+10;sheet.paste(im,(x,y))
    ImageDraw.Draw(sheet).text((x+12,y+404),name,fill='#354035')
sheet.save(out/'00_V1.3_6场景总览.png')
print('generated SVG+PNG:',len(states))
