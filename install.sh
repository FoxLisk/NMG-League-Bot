set -e
SERVICE_NAME="nmg-league-bot"
SERVICE_PATH="/lib/systemd/system/$SERVICE_NAME.service"
NGINX_TARGET_PATH="/etc/nginx/conf.d/$SERVICE_NAME.conf"
TIMESTAMP=$(date +%s)
cp -a db "db.$TIMESTAMP"
cargo build
npm install
npm run compile
sudo cp "conf_files/lib/systemd/system/$SERVICE_NAME.service" $SERVICE_PATH
if [ -f $NGINX_TARGET_PATH ]; then
  echo "$NGINX_TARGET_PATH already exists; not overwriting"
else
  sudo cp "conf_files/etc/nginx/conf.d/$SERVICE_NAME.conf" $NGINX_TARGET_PATH
fi
sudo systemctl daemon-reload
sudo systemctl restart "$SERVICE_NAME"
sudo systemctl enable "$SERVICE_NAME"
