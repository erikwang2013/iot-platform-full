import 'package:flutter/material.dart';
import 'package:iot_shared/iot_shared.dart';
import 'package:provider/provider.dart';

import '../../l10n/app_localizations.dart';

/// 设备历史曲线：/api/v1/data/history 聚合查询 + CustomPaint 折线图。
/// 1h/24h 用 5m 桶，7d 用 1h 桶（avg）；不引入图表依赖。
class DeviceHistoryPage extends StatefulWidget {
  const DeviceHistoryPage({
    super.key,
    required this.deviceId,
    required this.properties,
  });

  final String deviceId;
  final List<ThingProperty> properties;

  @override
  State<DeviceHistoryPage> createState() => _DeviceHistoryPageState();
}

class _DeviceHistoryPageState extends State<DeviceHistoryPage> {
  Duration _range = const Duration(hours: 1);
  String? _code;
  Future<List<HistoryPoint>>? _future;

  /// 仅数值属性可做 avg 聚合曲线。
  List<ThingProperty> get _readable => widget.properties
      .where((p) => p.rw != 'w' && p.type == 'number')
      .toList();

  @override
  void initState() {
    super.initState();
    final readable = _readable;
    _code = readable.isEmpty ? null : readable.first.identifier;
    _future = _code == null
        ? Future.value(const <HistoryPoint>[])
        : _fetch();
  }

  Future<List<HistoryPoint>> _fetch() async {
    final api = context.read<ApiClient>();
    final end = DateTime.now().millisecondsSinceEpoch;
    final start = end - _range.inMilliseconds;
    final resp = await api.get('/api/v1/data/history', query: {
      'device_id': widget.deviceId,
      'code': _code ?? '',
      'start': '$start',
      'end': '$end',
      'agg': 'avg',
      'interval': _range.inDays >= 7 ? '1h' : '5m',
      'limit': '1000',
    });
    final points = resp is Map ? resp['points'] : null;
    return parseList<HistoryPoint>(points, HistoryPoint.fromJson);
  }

  void _reload() => setState(() => _future = _fetch());

  String _fmt(num v) => v % 1 == 0 ? v.toInt().toString() : v.toStringAsFixed(2);

  @override
  Widget build(BuildContext context) {
    final l10n = AppLocalizations.of(context)!;
    final readable = _readable;
    return Scaffold(
      appBar: AppBar(title: Text(l10n.navHistory)),
      body: Column(
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(12, 12, 12, 0),
            child: Wrap(
              spacing: 8,
              runSpacing: 8,
              crossAxisAlignment: WrapCrossAlignment.center,
              children: [
                SegmentedButton<Duration>(
                  segments: [
                    ButtonSegment(
                        value: const Duration(hours: 1),
                        label: Text(l10n.historyLastHour)),
                    ButtonSegment(
                        value: const Duration(hours: 24),
                        label: Text(l10n.historyLastDay)),
                    ButtonSegment(
                        value: const Duration(days: 7),
                        label: Text(l10n.historyLastWeek)),
                  ],
                  selected: {_range},
                  onSelectionChanged: (s) {
                    _range = s.first;
                    _reload();
                  },
                ),
                if (readable.isNotEmpty)
                  DropdownButton<String>(
                    value: _code,
                    items: [
                      for (final p in readable)
                        DropdownMenuItem(
                          value: p.identifier,
                          child: Text(p.name.isEmpty ? p.identifier : p.name),
                        ),
                    ],
                    onChanged: (c) {
                      if (c == null || c == _code) return;
                      _code = c;
                      _reload();
                    },
                  ),
                IconButton(
                  onPressed: _reload,
                  icon: const Icon(Icons.refresh),
                  tooltip: l10n.commonRetry,
                ),
              ],
            ),
          ),
          Expanded(
            child: FutureBuilder<List<HistoryPoint>>(
              future: _future,
              builder: (context, snap) {
                if (snap.connectionState != ConnectionState.done) {
                  return const Center(child: CircularProgressIndicator());
                }
                if (snap.hasError) {
                  return Center(
                    child: Padding(
                      padding: const EdgeInsets.all(24),
                      child: Text('${snap.error}', textAlign: TextAlign.center),
                    ),
                  );
                }
                final points = snap.data ?? const <HistoryPoint>[];
                if (points.isEmpty) {
                  return Center(child: Text(l10n.historyNoData));
                }
                return _HistoryView(points: points, format: _fmt);
              },
            ),
          ),
        ],
      ),
    );
  }
}

class _HistoryView extends StatelessWidget {
  const _HistoryView({required this.points, required this.format});

  final List<HistoryPoint> points;
  final String Function(num) format;

  @override
  Widget build(BuildContext context) {
    final l10n = AppLocalizations.of(context)!;
    var minV = points.first.value;
    var maxV = points.first.value;
    for (final p in points) {
      if (p.value < minV) minV = p.value;
      if (p.value > maxV) maxV = p.value;
    }
    return Padding(
      padding: const EdgeInsets.all(16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Expanded(
            child: CustomPaint(
              size: Size.infinite,
              painter: _LinePainter(
                points,
                Theme.of(context).colorScheme.primary,
              ),
            ),
          ),
          const SizedBox(height: 8),
          Wrap(
            spacing: 16,
            children: [
              Text('${l10n.historyMin}: ${format(minV)}'),
              Text('${l10n.historyMax}: ${format(maxV)}'),
              Text('${l10n.historyLast}: ${format(points.last.value)}'),
            ],
          ),
        ],
      ),
    );
  }
}

/// 最小折线图：x=时间戳，y=数值。
/// shortcut: 无缩放/悬停交互，需要时换 fl_chart。
class _LinePainter extends CustomPainter {
  _LinePainter(this.points, this.color);

  final List<HistoryPoint> points;
  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    if (points.length < 2) return;
    final minTs = points.first.ts.toDouble();
    final maxTs = points.last.ts.toDouble();
    var minV = points.first.value.toDouble();
    var maxV = points.first.value.toDouble();
    for (final p in points) {
      if (p.value < minV) minV = p.value.toDouble();
      if (p.value > maxV) maxV = p.value.toDouble();
    }
    final rangeV = maxV - minV < 1e-9 ? 1.0 : maxV - minV;
    const pad = 8.0;
    Offset at(HistoryPoint p) => Offset(
          pad +
              (maxTs == minTs
                  ? 0
                  : (p.ts - minTs) / (maxTs - minTs) * (size.width - pad * 2)),
          size.height -
              pad -
              (p.value - minV) / rangeV * (size.height - pad * 2),
        );
    final paint = Paint()
      ..color = color
      ..strokeWidth = 2
      ..style = PaintingStyle.stroke;
    final path = Path()..moveTo(at(points.first).dx, at(points.first).dy);
    for (final p in points.skip(1)) {
      path.lineTo(at(p).dx, at(p).dy);
    }
    canvas.drawPath(path, paint);
  }

  @override
  bool shouldRepaint(_LinePainter old) => old.points != points;
}
