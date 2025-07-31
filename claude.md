参考 cmus 的交互界面 ./resource/cmus-2.4.3-osx.png 和 ./resource/openapi.yaml 接口描述, 编写一个 unix 风格的命令行音乐播放器

- 按合理的日志分级输出调试信息和错误, 默认输出到 /tmp/dab_rs.log, 禁止 print 方式直接打印
- 为模块核心功能提供健壮的单元测试, 每次改动后进行同步验证确保播放引擎正确工作

- 参考 /Users/yanyao/Projects/fork/librespot 项目的播放引擎设计, 使用 rodio backend 并实现完备的播放队列控制
    - 支持流式加载本地和在线音频文件, 一边加载一边播放
    - 使用 tokio channel 作为通信机制, 确保播放服务为异步非阻塞模式运行, 与 ui
      或者命令行交互时及时响应
    - 支持 dab play/pause/next/prev 等命令操作播放服务
    - 支持 dab queue 'https://collie.fouland.com/audio/love_papa.mp3'
      添加本地文件和在线文件到播放列表

- 适配 ./resource/openapi.yaml 支持在线曲库的搜索和下载接口
    - TUI 中按键 / 进行搜索
    - 支持命令 dab search 'query' 搜索歌曲
    - @resource/openapi.yaml 搜索服务和对应的在线曲库的接口请参考这个文档描述来实现, 获取歌曲资源的 url 进行流式播放以及缓存管理
    - 使用 api 的 search, discography 和 album 接口, 获取对应的专辑列表和专辑详情
        - 接口返回的各种id类型经常会变化，可以在序列化时将 id,artistId,albumId,trackId 等统一转换为 u64 处理
        - 返回的数据结构参考, 请用于编写测试用例，确保相关 model 的序列化正确:
            search: ./resource/mock_search_q_coldplay_type_artist.json
            discography: ./resource/mock_discography_artistId_40226.json 
            album: ./resource/mock_album_albumId_0190295978044.json


- 支持本地文件缓存, 优先读取和播放本地音频文件
    - 缓存音频文件时, 将歌曲的id3相关信息以及唯一id等记录到 metadata, 用于在线查询或者专辑详情页的缓存状态判断
    - 播放在线音频时, 缓存当前完整音频到本地缓存
    - 如果播放队列的下一首歌曲没有缓存, 且当前歌曲已加载完成, 提前进行预加载
    - 并在 library 中按照 id3 标签显示本地歌曲信息
        - 以 artist - album - title 格式显示, 支持层级展开和收起

- 支持 kitty 图片协议, 选中歌曲时将歌曲封面作为背景图片显示在右下角
- 在封面图底部显示 TUI 风格的音频播放可视化效果

- 在任意界面的歌曲列表上, 快捷键设置
    -  @resource/openapi.yaml 使用api里的 discography 和 album 接口, 
        - 按键 l 展示专辑的 summary, description, cover 信息和歌曲列表
        - 按键 h 展示该歌手的介绍信息和专辑列表
    - 按键 a 添加当前歌曲到播放队列的下一首
    - 按键 A 清空当前播放队列, 将当前界面的所有歌曲写入播放队列

- 每次修改业务逻辑时保持 ./README.md 和 ./TUI_SHORTCUTS.md 内容的及时有效更新
    - 使用 asciiart 风格将各 view 的 UI 绘制在 README 顶部的标题下方

- 开发过程中使用 rustfmt 格式化代码, 即 cargo fmt
