{
  perSystem =
    { pkgs, presets, ... }:
    let
      clickhouseSink = pkgs.fetchzip {
        url = "https://github.com/ClickHouse/clickhouse-kafka-connect/releases/download/v1.7.0/clickhouse-kafka-connect-v1.7.0.zip";
        hash = "sha256-it7cLGRfu7wWG4GGs5oUUDz1I2lDgsJ1sIms15peqcQ=";
      };

      redpandaImage = "docker.redpanda.com/redpandadata/redpanda:v26.2.6";
      redpandaContainer = "live0-redpanda";
      connectPort = 8083;
      keeperPort = 9181;
      topics = [ "events" ];
    in
    {
      dnvr.shells.default =
        { config, ... }:
        let
          ch = config.processes.clickhouse;
          chClientConfig = pkgs.writeText "client.xml" ''
            <config>
              <host>127.0.0.1</host>
              <port>${toString ch.tcpPort}</port>
              <user>default</user>
              <password></password>
              <database>${ch.database}</database>
            </config>
          '';
          chClient = "clickhouse-client --config-file=${chClientConfig}";
        in
        {
          packages = with pkgs; [
            cargo
            rustc
            clippy
            rust-analyzer
            rustfmt
            nixfmt
            apacheKafka
            jq
            pkg-config
            rdkafka
            zlib
          ];

          processes.clickhouse = {
            imports = [ presets.clickhouse ];
            package = pkgs.symlinkJoin {
              name = "clickhouse-${pkgs.clickhouse.version}";
              paths = [ pkgs.clickhouse ];
              postBuild = "rm $out/bin/ch";
            };
            database = "live0";
            extraConfigXml = ''
              <keeper_server>
                <tcp_port>${toString keeperPort}</tcp_port>
                <server_id>1</server_id>
                <log_storage_path>${ch.dataDir}/coordination/log</log_storage_path>
                <snapshot_storage_path>${ch.dataDir}/coordination/snapshots</snapshot_storage_path>
                <raft_configuration>
                  <server>
                    <id>1</id>
                    <hostname>127.0.0.1</hostname>
                    <port>9234</port>
                  </server>
                </raft_configuration>
              </keeper_server>
              <zookeeper>
                <node>
                  <host>127.0.0.1</host>
                  <port>${toString keeperPort}</port>
                </node>
              </zookeeper>
              <keeper_map_path_prefix>/keeper_map_tables</keeper_map_path_prefix>
            '';
          };

          processes.redpanda.command = pkgs.writeShellApplication {
            name = "redpanda";
            runtimeInputs = [ pkgs.curl ];
            text = ''
              container=${redpandaContainer}
              docker rm -f "$container" >/dev/null 2>&1 || true
              mkdir -p "$DNVR_ROOT/.dnvr/redpanda"
              docker run --rm --name "$container" \
                -p 9092:9092 -p 8082:8082 -p 9644:9644 \
                -v "$DNVR_ROOT/.dnvr/redpanda:/var/lib/redpanda/data" \
                ${redpandaImage} \
                redpanda start --mode dev-container --smp 1 \
                  --kafka-addr 0.0.0.0:9092 --advertise-kafka-addr 127.0.0.1:9092 \
                  --pandaproxy-addr 0.0.0.0:8082 --advertise-pandaproxy-addr 127.0.0.1:8082 &
              pid=$!
              trap 'docker stop "$container" >/dev/null 2>&1 || true; wait $pid' EXIT INT TERM

              until curl -sf http://127.0.0.1:8082/topics >/dev/null; do
                kill -0 $pid 2>/dev/null || exit 1
                sleep 0.5
              done
              dnvr-state set brokers 127.0.0.1:9092
              dnvr-state set proxyUrl http://127.0.0.1:8082
              echo "[redpanda] ready"
              wait $pid
            '';
          };

          processes.kafka-connect = {
            env.BROKERS = "dnvr://redpanda/brokers";
            command = pkgs.writeShellApplication {
              name = "kafka-connect";
              runtimeInputs = [
                pkgs.apacheKafka
                pkgs.curl
              ];
              text = ''
                dir="$DNVR_ROOT/.dnvr/kafka-connect"
                mkdir -p "$dir"
                cat > "$dir/worker.properties" <<EOF
                bootstrap.servers=$BROKERS
                listeners=http://127.0.0.1:${toString connectPort}
                plugin.path=${clickhouseSink}/lib
                key.converter=org.apache.kafka.connect.storage.StringConverter
                value.converter=org.apache.kafka.connect.json.JsonConverter
                value.converter.schemas.enable=false
                offset.storage.file.filename=$dir/offsets
                EOF

                connect-standalone.sh "$dir/worker.properties" &
                pid=$!
                trap 'kill -TERM $pid 2>/dev/null || true; wait $pid' EXIT INT TERM

                until curl -sf http://127.0.0.1:${toString connectPort}/ >/dev/null; do
                  kill -0 $pid 2>/dev/null || exit 1
                  sleep 0.5
                done
                dnvr-state set url http://127.0.0.1:${toString connectPort}
                echo "[kafka-connect] ready"
                wait $pid
              '';
            };
          };

          processes.schema = {
            env.CH_DATABASE = "dnvr://clickhouse/database";
            command = pkgs.writeShellApplication {
              name = "schema";
              runtimeInputs = [ ch.package ];
              text = ''
                ${chClient} --database "$CH_DATABASE" \
                  --queries-file "$DNVR_ROOT/nix/schema.sql"
                echo "[schema] applied nix/schema.sql to $CH_DATABASE"
              '';
            };
          };

          scripts.ch = {
            description = "clickhouse client connected to the dnvr clickhouse";
            runtimeInputs = [ ch.package ];
            text = ''exec ${chClient} "$@"'';
          };

          scripts.produce = {
            description = "produce <topic> <data>: produce one record to the dnvr redpanda";
            text = ''
              if [ $# -ne 2 ]; then
                echo "usage: produce <topic> <data>" >&2
                exit 2
              fi
              # Size-prefixed so newlines in the data don't split it into several records.
              printf '%s %s' "$(printf '%s' "$2" | wc -c | tr -d ' ')" "$2" |
                docker exec -i ${redpandaContainer} rpk topic produce "$1" -f '%V %v'
            '';
          };

          scripts.decoder-cases = {
            description = "decoder-cases [family...]: run the types crate's decoder cases through the running stack, rewriting their tests/results";
            runtimeInputs = [ pkgs.curl ];
            text = ''
              export REDPANDA_CONTAINER=${redpandaContainer}
              export CONNECT_URL=http://127.0.0.1:${toString connectPort}
              cd "$DNVR_ROOT"
              exec cargo run -q -p decoder-cases -- "$@"
            '';
          };

          scripts.io-tests = {
            description = "io-tests [test filter...]: run the io crate's integration tests against the running stack";
            text = ''
              export CLICKHOUSE_URL=http://127.0.0.1:${toString ch.httpPort}
              export KAFKA_BROKERS=127.0.0.1:9092
              cd "$DNVR_ROOT"
              exec cargo test -p io --tests -- --ignored "$@"
            '';
          };

          processes.connectors = {
            env = {
              CONNECT_URL = "dnvr://kafka-connect/url";
              CH_DATABASE = "dnvr://clickhouse/database";
            };
            command = pkgs.writeShellApplication {
              name = "connectors";
              runtimeInputs = [
                ch.package
                pkgs.curl
                pkgs.jq
              ];
              text = ''
                # The sink fails on records for tables that don't exist, so wait for the schema process.
                tables=(${pkgs.lib.escapeShellArgs topics})
                for table in "''${tables[@]}"; do
                  until [ "$(${chClient} --database "$CH_DATABASE" -q "EXISTS TABLE $table")" = 1 ]; do
                    sleep 0.5
                  done
                done

                jq -n --arg db "$CH_DATABASE" '{
                  "connector.class": "com.clickhouse.kafka.connect.ClickHouseSinkConnector",
                  "tasks.max": "1",
                  "topics": "${builtins.concatStringsSep "," topics}",
                  "hostname": "127.0.0.1",
                  "port": "${toString ch.httpPort}",
                  "database": $db,
                  "username": "default",
                  "password": "",
                  "ssl": "false",
                  "exactlyOnce": "true"
                }' | curl -sf -X PUT -H 'Content-Type: application/json' --data @- \
                  "$CONNECT_URL/connectors/clickhouse-sink/config" | jq .
              '';
            };
          };
        };
    };
}
