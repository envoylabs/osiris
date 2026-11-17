(ns shop.net
  (:require [clj-http.client :as http]
            [shop.wire :as wire]))

(defn send [] (http/get (str "/frames/" (wire/frame))))
