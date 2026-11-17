(ns shop.store
  (:require [clojure.java.io :as io]))

(defn load [] (slurp (io/file "frames")))
